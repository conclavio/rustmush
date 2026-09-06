//! E3c acceptance (doc 08b §5.1/§5.3): `mush-sim` as the economy's custody
//! provider for interior vaults, driven through the real `Engine` via
//! `TestWorld` with the production module shape — org, **then sim, then
//! economy** (the E3c wiring order; see `mush-server/src/modules.rs`).
//!
//! Four things are proved here, all of them cross-crate:
//!
//! 1. A deposit/withdraw against an interior vault moves the unit through a
//!    `Transition` and lands in the SAME `WriteBatch` as the journal entry that
//!    records it.
//! 2. Every refusal path — a non-participant, a sim-owned object offered to a
//!    container vault, an invalid backing — mutates **nothing**.
//! 3. Destruction and reboot fail in the safe direction: the vault freezes,
//!    title survives, and a legitimate interior deposit does **not** boot as a
//!    custody mismatch (which is exactly what the module-order fix buys).
//! 4. **The operation axis is independent of title** (D5): `@asset/*` never
//!    touches a station seat or a control epoch, and `@spatial/assign` never
//!    touches an economic assignment or owner.

use std::collections::HashMap;
use std::sync::Arc;

use mush_core::{
    Audience, BehaviorBinding, BehaviorId, Effects, GOD, GameModule, LIMBO, ModuleBootData,
    ModuleWrite, ObjId, ObjKind, Outbox, WorldEvent,
};
use mush_economy::{EconomySnapshot, EconomyStore, VaultId, VaultStatus};
use mush_proto::OutputEvent;
use mush_sim::{
    InteriorCustodyProvider, PlacementRecord, SimulationModule, SpatialParent, SpatialState,
    StationId, UnplacedInfo,
};
use mush_testkit::{TestWorld, text_of};
use smol_str::SmolStr;

// ---- rig -------------------------------------------------------------------

/// The production order: org (economy authorizes through it), sim, then
/// economy — so the economy's boot custody reconciliation sees loaded sim state.
fn modules() -> Vec<Box<dyn GameModule>> {
    vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(SimulationModule::new()),
        Box::new(mush_economy::EconomyModule::with_custody(vec![Arc::new(
            InteriorCustodyProvider,
        )])),
    ]
}
/// The *pre*-E3c order, kept only to pin why the fix matters.
fn modules_economy_first() -> Vec<Box<dyn GameModule>> {
    vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(mush_economy::EconomyModule::with_custody(vec![Arc::new(
            InteriorCustodyProvider,
        )])),
        Box::new(SimulationModule::new()),
    ]
}

fn boot() -> TestWorld {
    TestWorld::with_modules(modules())
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
fn ok(tw: &mut TestWorld, line: &str) -> String {
    let events = tw.run(GOD, line);
    let t = texts(&events);
    assert!(!has_error(&events), "{line:?} was refused: {t}");
    t
}
fn refused(tw: &mut TestWorld, line: &str) -> String {
    let events = tw.run(GOD, line);
    let t = texts(&events);
    assert!(has_error(&events), "{line:?} was accepted: {t}");
    t
}
fn bind(tw: &mut TestWorld, obj: ObjId, behavior: &str) {
    tw.world_mut()
        .attach_behavior(
            obj,
            BehaviorId::new(behavior).unwrap(),
            BehaviorBinding {
                schema_version: 1,
                attached_by: GOD,
                attached_at: 0,
            },
        )
        .unwrap();
}
fn loc(tw: &TestWorld, obj: ObjId) -> Option<ObjId> {
    tw.world().get(obj).and_then(|o| o.location)
}
fn sim(tw: &TestWorld) -> &SpatialState {
    tw.world()
        .module_state::<SpatialState>()
        .expect("sim state")
}
fn store(tw: &TestWorld) -> &EconomyStore {
    tw.world()
        .module_state::<EconomyStore>()
        .expect("economy store")
}
fn snap(tw: &TestWorld) -> &EconomySnapshot {
    store(tw).snapshot()
}
fn journal_len(tw: &TestWorld) -> usize {
    store(tw).transactions().count()
}
fn seats(tw: &TestWorld, unit: ObjId) -> Vec<(StationId, ObjId)> {
    sim(tw)
        .assignments
        .get(&unit)
        .map(|a| a.seats.iter().map(|(s, x)| (s.clone(), x.actor)).collect())
        .unwrap_or_default()
}

/// The deterministic object fixture (ids follow creation order after Limbo #0
/// and God #1), so a cold reboot can rebuild the same world.
struct Fx {
    field: ObjId,
    hangar: ObjId,
    atlas: ObjId,
    crate_: ObjId,
    chest: ObjId,
}

fn fixture(tw: &mut TestWorld) -> Fx {
    let field = tw
        .world_mut()
        .create(ObjKind::Room, "Field", GOD, None)
        .unwrap();
    let hangar = tw
        .world_mut()
        .create(ObjKind::Room, "Hangar", GOD, None)
        .unwrap();
    let atlas = tw
        .world_mut()
        .create(ObjKind::Thing, "Atlas", GOD, Some(LIMBO))
        .unwrap();
    let crate_ = tw
        .world_mut()
        .create(ObjKind::Thing, "Crate", GOD, Some(LIMBO))
        .unwrap();
    let chest = tw
        .world_mut()
        .create(ObjKind::Thing, "Chest", GOD, Some(LIMBO))
        .unwrap();
    bind(tw, field, "sim.domain");
    bind(tw, hangar, "sim.interior");
    bind(tw, atlas, "sim.participant");
    Fx {
        field,
        hangar,
        atlas,
        crate_,
        chest,
    }
}

/// Domain #0 on Field, Hangar fixed in it, and vault #0 backed by the Hangar.
fn hangar_vault(tw: &mut TestWorld, fx: &Fx) {
    ok(tw, &format!("@domain/create #{}=sim.passive", fx.field.0));
    ok(tw, &format!("@spatial/space #{}=fixed:0", fx.hangar.0));
    let t = ok(tw, &format!("@vault/create bay=interior:#{}", fx.hangar.0));
    assert!(t.contains("Created vault #0"), "{t}");
}

/// Drain every queued module op once and fold it per namespace (a later
/// `JsonSet` wins, a `Del` removes) — the kv each module would boot from.
fn fold_all(tw: &mut TestWorld) -> HashMap<SmolStr, ModuleBootData> {
    let mut kv: HashMap<SmolStr, Vec<(SmolStr, serde_json::Value)>> = HashMap::new();
    for op in tw.take_module_ops() {
        let ns = kv.entry(op.ns.clone()).or_default();
        match op.op {
            ModuleWrite::JsonSet { key, value } => {
                ns.retain(|(k, _)| *k != key);
                ns.push((key, value));
            }
            ModuleWrite::Del { key } => ns.retain(|(k, _)| *k != key),
            ModuleWrite::StreamAppend { .. } => {}
        }
    }
    kv.into_iter()
        .map(|(ns, kv)| {
            (
                ns,
                ModuleBootData {
                    kv,
                    streams: HashMap::new(),
                    ..Default::default()
                },
            )
        })
        .collect()
}

/// A **cold** reboot: a brand-new engine with `mods` registered, the same world
/// objects rebuilt in the same order, then the persisted data handed back. This
/// is the shape that exposes module ORDER — an in-place `modules_loaded` leaves
/// the previous `SpatialState` in its slot and hides it.
fn cold_reboot(
    mods: Vec<Box<dyn GameModule>>,
    boot_data: HashMap<SmolStr, ModuleBootData>,
) -> (TestWorld, Fx) {
    let mut tw = TestWorld::with_modules(mods);
    let fx = fixture(&mut tw);
    tw.modules_loaded(boot_data);
    (tw, fx)
}

// ======================================================================
// The happy path
// ======================================================================

#[test]
fn interior_deposit_and_withdraw_are_atomic_with_the_journal() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    hangar_vault(&mut tw, &fx);
    ok(&mut tw, &format!("@asset/title #{}=#{}", fx.atlas.0, GOD.0));
    tw.drain_write_batch();

    // Deposit: the unit becomes an interior placement, and its projection
    // follows (design D1: `Interior { space }` projects to the space itself).
    ok(&mut tw, &format!("@vault/deposit #{}=0", fx.atlas.0));
    assert!(
        matches!(sim(&tw).parent_of(fx.atlas), SpatialParent::Interior { space, slot: None } if space == fx.hangar),
        "the deposit did not place the unit inside the hangar: {:?}",
        sim(&tw).parent_of(fx.atlas)
    );
    assert_eq!(
        loc(&tw, fx.atlas),
        Some(fx.hangar),
        "the projection did not follow the placement"
    );
    tw.drain_write_batch()
        .asserts_atomic_with_ledger(fx.atlas, "economy", "journal");

    // The vault reports it live, through the provider (never a stored list).
    let t = ok(&mut tw, "@vault/show 0");
    assert!(t.contains(&format!("holds #{}", fx.atlas.0)), "{t}");

    // Withdraw: out of the simulation entirely, into the actor's hands.
    ok(&mut tw, &format!("@vault/withdraw #{}=0", fx.atlas.0));
    assert_eq!(
        loc(&tw, fx.atlas),
        Some(GOD),
        "the unit did not land on the actor"
    );
    assert!(
        matches!(sim(&tw).parent_of(fx.atlas), SpatialParent::Unplaced),
        "a withdrawn unit is no longer a placed simulation citizen"
    );
    assert!(
        !sim(&tw).is_quarantined(fx.atlas),
        "a withdrawal is not a quarantine"
    );
    tw.drain_write_batch()
        .asserts_atomic_with_ledger(fx.atlas, "economy", "journal");

    // Title never moved (D5: custody is not ownership).
    assert_eq!(
        snap(&tw).asset(fx.atlas).expect("titled").title_owner,
        mush_org::PrincipalId::Object(GOD)
    );
}

#[test]
fn a_withdrawn_unit_moves_and_carries_freely_again() {
    // `keep: true` leaves `Obj.location` on the space and clears the placement,
    // so the very next ordinary move must not be vetoed by the placement gate.
    let mut tw = boot();
    let fx = fixture(&mut tw);
    hangar_vault(&mut tw, &fx);
    ok(&mut tw, &format!("@vault/deposit #{}=0", fx.atlas.0));
    ok(&mut tw, &format!("@vault/withdraw #{}=0", fx.atlas.0));
    // A plain object again: the world may move it with no simulation involved.
    tw.world_mut()
        .move_to(fx.atlas, fx.field)
        .expect("a withdrawn unit moves freely");
    assert_eq!(loc(&tw, fx.atlas), Some(fx.field));
}

// ======================================================================
// Refusals that must mutate nothing
// ======================================================================

#[test]
fn a_non_participant_deposit_is_refused_and_mutates_nothing() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    hangar_vault(&mut tw, &fx);
    let before_loc = loc(&tw, fx.crate_);
    let before_txs = journal_len(&tw);
    tw.take_module_ops();

    let t = refused(&mut tw, &format!("@vault/deposit #{}=0", fx.crate_.0));
    assert!(
        t.contains("sim.participant"),
        "the refusal must name the missing behavior: {t}"
    );
    assert_eq!(
        loc(&tw, fx.crate_),
        before_loc,
        "a refused deposit moved the object"
    );
    assert!(
        !sim(&tw).placements.contains_key(&fx.crate_),
        "a refused deposit left a placement record"
    );
    assert_eq!(
        journal_len(&tw),
        before_txs,
        "a refused deposit journaled a transaction"
    );
    assert!(
        tw.take_module_ops().is_empty(),
        "a refused deposit queued persists"
    );
    // ...and the provider attached nothing on its way out.
    assert!(
        !tw.world()
            .has_behavior(fx.crate_, &BehaviorId::new("sim.participant").unwrap()),
        "the provider must never attach `sim.participant` itself"
    );
}

#[test]
fn a_sim_placed_unit_is_never_routed_to_a_container_vault() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    ok(
        &mut tw,
        &format!("@domain/create #{}=sim.passive", fx.field.0),
    );
    ok(
        &mut tw,
        &format!("@vault/create strongbox=container:#{}", fx.chest.0),
    );
    ok(&mut tw, &format!("@spatial/place/domain #{}=0", fx.atlas.0));
    // Stand where the unit stands, so custody STANDING is satisfied and the
    // refusal can only come from the routing/claims rule.
    tw.world_mut()
        .move_to(GOD, fx.field)
        .expect("staff walks to the field");
    tw.world_mut()
        .move_to(fx.chest, fx.field)
        .expect("the chest is here too");
    let before_loc = loc(&tw, fx.atlas);
    let before_txs = journal_len(&tw);
    tw.take_module_ops();

    let t = refused(&mut tw, &format!("@vault/deposit #{}=0", fx.atlas.0));
    assert!(
        t.contains("sim"),
        "the refusal should name the claiming system: {t}"
    );
    assert_eq!(loc(&tw, fx.atlas), before_loc);
    assert_eq!(journal_len(&tw), before_txs);
    assert!(tw.take_module_ops().is_empty());

    // The same rule at creation time: a sim-owned object cannot back a
    // *container* vault (its own interior backing is a different question).
    refused(
        &mut tw,
        &format!("@vault/create hull=container:#{}", fx.atlas.0),
    );
}

#[test]
fn a_room_with_no_space_record_cannot_back_an_interior_vault() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    // `sim.interior` is bound but `@spatial/space` never ran: no record, so the
    // provider answers `Some(false)` and the vault is refused.
    let t = refused(
        &mut tw,
        &format!("@vault/create bay=interior:#{}", fx.hangar.0),
    );
    assert!(t.contains("cannot back a vault"), "{t}");
    assert_eq!(snap(&tw).vaults().count(), 0);
}

#[test]
fn a_quarantined_space_invalidates_its_vault_and_refuses_deposits() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    hangar_vault(&mut tw, &fx);
    // Quarantine is normally reached through boot validation or the destroy
    // backstop; planting the record is the direct way to exercise the
    // predicate `backing_valid` reads.
    tw.world_mut()
        .module_state_mut::<SpatialState>()
        .expect("sim state")
        .placements
        .insert(
            fx.hangar,
            PlacementRecord {
                object: fx.hangar,
                parent: SpatialParent::Unplaced,
                revision: 1,
                unplaced: Some(UnplacedInfo {
                    reason: "test quarantine".into(),
                    by: None,
                    at: 0,
                    previous: None,
                    quarantined: true,
                }),
            },
        );
    assert!(sim(&tw).is_quarantined(fx.hangar));
    let before_txs = journal_len(&tw);
    tw.take_module_ops();

    refused(&mut tw, &format!("@vault/deposit #{}=0", fx.atlas.0));
    assert!(
        !sim(&tw).placements.contains_key(&fx.atlas),
        "a refused deposit placed the unit"
    );
    assert_eq!(loc(&tw, fx.atlas), Some(LIMBO));
    assert_eq!(journal_len(&tw), before_txs);
    assert!(tw.take_module_ops().is_empty());
}

// ======================================================================
// Destruction + reboot: fail in the safe direction
// ======================================================================

#[test]
fn destroying_the_space_freezes_the_vault_and_leaves_title_standing() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    hangar_vault(&mut tw, &fx);
    let pat = tw.spawn_player("Pat");
    ok(&mut tw, &format!("@asset/title #{}=#{}", fx.atlas.0, pat.0));
    ok(&mut tw, &format!("@vault/deposit #{}=0", fx.atlas.0));

    // Programmatic destruction (the `@destroy` path is refused by both gates).
    tw.world_mut()
        .destroy(fx.hangar)
        .expect("destroy the space");
    let mut out = Outbox::new();
    let mut fxx = Effects::new();
    tw.engine_mut().emit_world_event(
        &WorldEvent::ObjDestroyed { obj: fx.hangar },
        &mut out,
        &mut fxx,
    );

    assert_eq!(
        snap(&tw).vault(VaultId(0)).expect("vault").status,
        VaultStatus::Frozen,
        "a vault whose backing died must freeze, never vanish"
    );
    let asset = snap(&tw)
        .asset(fx.atlas)
        .expect("the title survives its custodian");
    assert_eq!(
        asset.title_owner,
        mush_org::PrincipalId::Object(pat),
        "destruction moved title"
    );
    assert!(
        !asset.frozen,
        "the unit itself did not die; its title stays usable"
    );
    // Sim quarantined the orphaned occupant rather than losing it.
    assert!(
        sim(&tw).is_quarantined(fx.atlas),
        "the occupant of a destroyed space is quarantined"
    );
}

#[test]
fn a_reboot_with_interior_deposits_freezes_nothing() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    hangar_vault(&mut tw, &fx);
    ok(&mut tw, &format!("@asset/title #{}=#{}", fx.atlas.0, GOD.0));
    ok(&mut tw, &format!("@vault/deposit #{}=0", fx.atlas.0));
    let data = fold_all(&mut tw);

    let (tw2, fx2) = cold_reboot(modules(), data);
    assert_eq!(fx.atlas, fx2.atlas, "the fixture rebuilds the same ids");
    assert!(
        !store(&tw2).is_frozen(),
        "a clean reboot must not freeze the economy"
    );
    let asset = snap(&tw2)
        .asset(fx2.atlas)
        .expect("the title replayed from the journal");
    assert!(
        !asset.frozen,
        "a legitimate interior deposit booted as a custody mismatch: {:?}",
        asset.frozen_reason
    );
    assert_eq!(
        snap(&tw2).vault(VaultId(0)).expect("vault").status,
        VaultStatus::Active,
        "the interior backing is still valid, so the vault stays Active"
    );
    assert!(
        matches!(sim(&tw2).parent_of(fx2.atlas), SpatialParent::Interior { space, .. } if space == fx2.hangar),
        "the placement replayed"
    );
    assert_eq!(
        loc(&tw2, fx2.atlas),
        Some(fx2.hangar),
        "sim boot reconciled the projection"
    );
}

#[test]
fn booting_the_economy_before_the_simulation_is_what_the_order_fix_prevents() {
    // The reason `EconomyModule` moved after `SimulationModule` (§4.10 amended):
    // `SpatialState` is installed at REGISTRATION, so an economy that boots
    // first gets a definite `Some(false)` from a module that has simply not
    // read its documents yet — and fails closed on a lie.
    let mut tw = boot();
    let fx = fixture(&mut tw);
    hangar_vault(&mut tw, &fx);
    ok(&mut tw, &format!("@asset/title #{}=#{}", fx.atlas.0, GOD.0));
    ok(&mut tw, &format!("@vault/deposit #{}=0", fx.atlas.0));
    let data = fold_all(&mut tw);

    let (tw2, fx2) = cold_reboot(modules_economy_first(), data);
    let asset = snap(&tw2).asset(fx2.atlas).expect("titled");
    assert!(
        asset.frozen,
        "the wrong order is supposed to mis-reconcile — that is the whole point"
    );
    assert_eq!(asset.frozen_reason.as_deref(), Some("custody-mismatch"));
    assert_eq!(
        snap(&tw2).vault(VaultId(0)).expect("vault").status,
        VaultStatus::Frozen
    );
}

// ======================================================================
// The operation axis (D5): title, custody, and control are independent
// ======================================================================

#[test]
fn asset_verbs_never_touch_stations_and_station_verbs_never_touch_title() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    let pat = tw.spawn_player("Pat");
    let ann = tw.spawn_player("Ann");
    let bob = tw.spawn_player("Bob");
    let quinn = tw.spawn_player("Quinn");
    ok(
        &mut tw,
        &format!("@domain/create #{}=sim.passive", fx.field.0),
    );
    ok(&mut tw, &format!("@spatial/place/domain #{}=0", fx.atlas.0));
    ok(
        &mut tw,
        &format!("@spatial/station #{}=pilot,pilot", fx.atlas.0),
    );
    ok(&mut tw, &format!("@asset/title #{}=#{}", fx.atlas.0, pat.0));
    ok(
        &mut tw,
        &format!("@asset/assign #{}=#{} field duty", fx.atlas.0, ann.0),
    );

    // --- crewing the unit leaves the economic record alone ---
    let owner_before = snap(&tw)
        .asset(fx.atlas)
        .expect("titled")
        .title_owner
        .clone();
    let assignment_before = snap(&tw)
        .asset(fx.atlas)
        .expect("titled")
        .assignment
        .clone();
    ok(
        &mut tw,
        &format!("@spatial/assign #{}=pilot,#{}", fx.atlas.0, ann.0),
    );
    let asset = snap(&tw).asset(fx.atlas).expect("titled");
    assert_eq!(
        asset.title_owner, owner_before,
        "crewing a unit moved its title"
    );
    assert_eq!(
        asset.assignment, assignment_before,
        "crewing a unit rewrote its economic assignment"
    );

    // --- and the economic verbs leave the control surface alone ---
    let seats_before = seats(&tw, fx.atlas);
    let gen_before = sim(&tw).control_generation(fx.atlas);
    assert_eq!(seats_before.len(), 1, "the unit is crewed");

    ok(
        &mut tw,
        &format!("@asset/assign #{}=#{} depot duty", fx.atlas.0, bob.0),
    );
    assert_eq!(
        seats(&tw, fx.atlas),
        seats_before,
        "@asset/assign evicted the crew"
    );
    assert_eq!(
        sim(&tw).control_generation(fx.atlas),
        gen_before,
        "@asset/assign bumped the control epoch"
    );

    ok(
        &mut tw,
        &format!("@asset/transfer #{}=#{}", fx.atlas.0, quinn.0),
    );
    assert_eq!(
        snap(&tw).asset(fx.atlas).expect("titled").title_owner,
        mush_org::PrincipalId::Object(quinn),
        "the transfer did not move title"
    );
    assert_eq!(
        seats(&tw, fx.atlas),
        seats_before,
        "a new owner has no eviction verb in E3 (D5)"
    );
    assert_eq!(
        sim(&tw).control_generation(fx.atlas),
        gen_before,
        "@asset/transfer bumped the control epoch"
    );
    // The unit also did not move (title is not custody).
    assert_eq!(loc(&tw, fx.atlas), Some(fx.field));
}

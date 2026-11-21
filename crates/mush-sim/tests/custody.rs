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
    Audience, BehaviorBinding, BehaviorId, Effects, GOD, GameModule, LIMBO, ModuleBootData, ObjId,
    ObjKind, Outbox, WorldEvent,
};
use mush_economy::{EconomySnapshot, EconomyStore, VaultId, VaultStatus};
use mush_proto::OutputEvent;
use mush_sim::{
    InteriorCustodyProvider, PlacementRecord, SimulationModule, SpatialParent, SpatialState,
    StationId, UnplacedInfo,
};
use mush_testkit::{authoritative::AcknowledgedWorld as TestWorld, text_of};
use smol_str::SmolStr;

#[path = "support/space_command.rs"]
mod space_command;

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
    let events = space_command::run(tw, GOD, line);
    let t = texts(&events);
    assert!(!has_error(&events), "{line:?} was refused: {t}");
    t
}
fn refused(tw: &mut TestWorld, line: &str) -> String {
    let events = space_command::run(tw, GOD, line);
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
    tw.persisted_boot()
}

/// A **cold** reboot: a brand-new engine with `mods` registered, the same world
/// objects rebuilt in the same order, then the persisted data handed back. This
/// is the shape that exposes module ORDER — an in-place `modules_loaded` leaves
/// the previous `SpatialState` in its slot and hides it.
fn cold_reboot(
    source: &TestWorld,
    mods: Vec<Box<dyn GameModule>>,
    boot_data: HashMap<SmolStr, ModuleBootData>,
) -> (TestWorld, Fx) {
    let mut tw = TestWorld::with_modules(mods);
    tw.adopt_store(source);
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

    let (tw2, fx2) = cold_reboot(&tw, modules(), data);
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

    let (tw2, fx2) = cold_reboot(&tw, modules_economy_first(), data);
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

// ---- doc 18 cut IC3: a passive domain is not a window ------------------------

/// **A passive domain opens while every window is open** (doc 18 §4.4, §9 risk
/// 1; cut IC3): `open_passive_domain` writes `window: None` and `windows()`
/// never counts it, so `MAX_WINDOWS` no longer bounds it. Eight window records
/// are installed straight into the sim's slot (this rig composes no atlas to
/// open one through), then `@domain/create … = sim.passive` on a ninth room
/// succeeds and the window count is unchanged.
#[test]
fn ic3_a_passive_domain_opens_with_every_window_open() {
    use mush_sim::{
        DomainClock, DomainRecord, DomainStatus, MAX_WINDOWS, WindowBox, WindowOpener, WindowRec,
        passive_definition_ref,
    };

    let mut tw = boot();
    let rooms: Vec<ObjId> = (0..=MAX_WINDOWS)
        .map(|n| {
            tw.world_mut()
                .create(ObjKind::Room, &format!("Room {n}"), GOD, None)
                .expect("a room")
        })
        .collect();
    {
        let st = tw
            .world_mut()
            .module_state_mut::<SpatialState>()
            .expect("sim state");
        for host in rooms.iter().take(MAX_WINDOWS) {
            let id = st.alloc_domain_id().expect("an id");
            st.domains.insert(
                id,
                DomainRecord {
                    id,
                    host: *host,
                    driver: passive_definition_ref(),
                    status: DomainStatus::Active,
                    clock: DomainClock::Passive,
                    presentation_location: *host,
                    revision: 1,
                    window: Some(WindowRec {
                        region: SmolStr::new("region.test"),
                        surfaces: Vec::new(),
                        bbox: WindowBox {
                            x0: 0,
                            y0: 0,
                            x1: 1_000,
                            y1: 1_000,
                        },
                        cell_cm: 100,
                        composited_from: Vec::new(),
                        features_seen: 0,
                        opened_by: WindowOpener::Staff { by: GOD },
                        revision: 1,
                    }),
                },
            );
        }
    }
    assert_eq!(mush_sim::windows(tw.world()).count(), MAX_WINDOWS);
    let spare = *rooms.last().expect("the ninth room");
    ok(&mut tw, &format!("@behavior/add #{}=sim.domain", spare.0));
    let out = ok(&mut tw, &format!("@domain/create #{}=sim.passive", spare.0));
    assert!(out.contains("Created domain #"), "{out}");
    assert_eq!(
        mush_sim::windows(tw.world()).count(),
        MAX_WINDOWS,
        "a passive domain holds no window"
    );
}

// ======================================================================
// Pass 2 security regressions (P2-MOD-4, P2-MOD-5, P2-MOD-23, P2-MOD-24)
// ======================================================================

/// **P2-MOD-4**: Depositing a ship/carrier with anchored interior spaces (or a
/// container holding an actor or vault backing) into another `SpatialInterior`
/// vault is refused, and `interior_contents`/`interior_holder`/`move_out` never
/// absorb nested vault backings, multi-room interiors, or actors (`Player`/`Npc`).
#[test]
fn p2_mod_4_refuses_transitive_vault_nesting_and_actor_abduction() {
    use mush_economy::{CustodyProvider, CustodyRef};

    let mut tw = boot();
    let fx = fixture(&mut tw);
    hangar_vault(&mut tw, &fx); // Vault #0 backed by fx.hangar (V_A)

    // Create a carrier C with an anchored stowage vault V_B, a multi-room cab,
    // an item X inside V_B, a Player P and an NPC N inside cab.
    let carrier = tw
        .world_mut()
        .create(ObjKind::Thing, "Carrier", GOD, Some(LIMBO))
        .unwrap();
    let stowage = tw
        .world_mut()
        .create(ObjKind::Room, "Stowage", GOD, None)
        .unwrap();
    let cab = tw
        .world_mut()
        .create(ObjKind::Room, "Cab", GOD, None)
        .unwrap();
    let cargo = tw
        .world_mut()
        .create(ObjKind::Thing, "Escrow Cargo", GOD, Some(LIMBO))
        .unwrap();
    let captive_player = tw.spawn_player("VictimPlayer");
    let captive_npc = tw
        .world_mut()
        .create(ObjKind::Npc, "VictimNpc", GOD, Some(LIMBO))
        .unwrap();
    bind(&mut tw, carrier, "sim.participant");
    bind(&mut tw, stowage, "sim.interior");
    bind(&mut tw, cab, "sim.interior");
    bind(&mut tw, cargo, "sim.participant");
    bind(&mut tw, captive_player, "sim.participant");
    bind(&mut tw, captive_npc, "sim.participant");

    ok(
        &mut tw,
        &format!("@spatial/space #{}=moving:#{}", stowage.0, carrier.0),
    );
    ok(
        &mut tw,
        &format!("@spatial/space #{}=moving:#{}", cab.0, carrier.0),
    );
    // Vault #1 backed by carrier's stowage (V_B), holding `cargo`.
    let t = ok(
        &mut tw,
        &format!("@vault/create hold=interior:#{}", stowage.0),
    );
    assert!(t.contains("Created vault #1"), "{t}");
    ok(&mut tw, &format!("@vault/deposit #{}=1", cargo.0));
    ok(
        &mut tw,
        &format!("@spatial/place/interior #{}=#{}", captive_player.0, cab.0),
    );
    ok(
        &mut tw,
        &format!("@spatial/place/interior #{}=#{}", captive_npc.0, cab.0),
    );

    // 1. Depositing carrier C (which carries anchored spaces) into V_A (#0) is refused.
    let before_txs = journal_len(&tw);
    let err = refused(&mut tw, &format!("@vault/deposit #{}=0", carrier.0));
    assert!(
        err.contains("anchored interior spaces"),
        "carrier deposit must be refused: {err}"
    );
    assert_eq!(journal_len(&tw), before_txs);
    assert!(!sim(&tw).is_placed(carrier));

    // 2. Depositing a container holding an actor or vault backing physically is refused.
    let box_with_actor = tw
        .world_mut()
        .create(ObjKind::Thing, "Box", GOD, Some(LIMBO))
        .unwrap();
    bind(&mut tw, box_with_actor, "sim.participant");
    let npc_inside = tw
        .world_mut()
        .create(ObjKind::Npc, "Stowaway", GOD, Some(box_with_actor))
        .unwrap();
    assert_eq!(loc(&tw, npc_inside), Some(box_with_actor));
    let err = refused(&mut tw, &format!("@vault/deposit #{}=0", box_with_actor.0));
    assert!(
        err.contains("contains"),
        "container holding an actor must be refused: {err}"
    );

    // 3. Even if staff forcibly places `carrier` inside V_A's space (hangar),
    // V_A must NEVER claim custody over nested vault V_B's `cargo` or actors in `cab`,
    // and withdrawing them through V_A must be refused.
    ok(
        &mut tw,
        &format!("@spatial/place/interior #{}=#{}", carrier.0, fx.hangar.0),
    );
    let provider = InteriorCustodyProvider;
    let va_ref = CustodyRef::SpatialInterior(fx.hangar);
    let va_contents = provider.contents(tw.world(), &va_ref).expect("contents");
    assert!(
        !va_contents.contains(&cargo),
        "V_A must not claim V_B cargo"
    );
    assert!(
        !va_contents.contains(&captive_player),
        "V_A must not claim Player"
    );
    assert!(
        !va_contents.contains(&captive_npc),
        "V_A must not claim NPC"
    );
    assert_eq!(provider.holds(tw.world(), &va_ref, cargo), Some(false));
    assert_eq!(
        provider.holds(tw.world(), &va_ref, captive_player),
        Some(false)
    );
    assert_eq!(
        provider.holds(tw.world(), &va_ref, captive_npc),
        Some(false)
    );
    refused(&mut tw, &format!("@vault/withdraw #{}=0", cargo.0));
    refused(&mut tw, &format!("@vault/withdraw #{}=0", captive_player.0));
    refused(&mut tw, &format!("@vault/withdraw #{}=0", captive_npc.0));
    assert_eq!(loc(&tw, cargo), Some(stowage));
    assert_eq!(loc(&tw, captive_player), Some(cab));
    assert_eq!(loc(&tw, captive_npc), Some(cab));

    // Even direct `move_out` on V_A must refuse actors and nested-vault occupants.
    {
        let mut out = Outbox::new();
        let eng = tw.engine_mut();
        let mut ctx = mush_core::Ctx {
            w: &mut eng.world,
            actor: GOD,
            switches: mush_core::SwitchSet::default(),
            args: mush_core::ParsedArgs::default(),
            out: &mut out,
            reg: &eng.registry,
            module: Some(SmolStr::new("economy")),
            typed_state: mush_core::ModuleActionState::unavailable(),
        };
        assert!(
            provider
                .move_out(&mut ctx, GOD, captive_player, &va_ref, GOD)
                .is_err(),
            "direct move_out must refuse Player"
        );
        assert!(
            provider
                .move_out(&mut ctx, GOD, captive_npc, &va_ref, GOD)
                .is_err(),
            "direct move_out must refuse Npc"
        );
        assert!(
            provider
                .move_out(&mut ctx, GOD, cargo, &va_ref, GOD)
                .is_err(),
            "direct move_out must refuse nested vault cargo"
        );
    }
}

/// **P2-MOD-5**: `InteriorCustodyProvider::move_into` refuses already-placed
/// objects (another vault, a domain, a mount), quarantined objects, and
/// under-way carriers — preventing cross-vault theft past `custody_lock`/Frozen.
#[test]
fn p2_mod_5_refuses_depositing_already_placed_or_quarantined_objects() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    hangar_vault(&mut tw, &fx); // Vault #0 backed by fx.hangar (V_A)

    let hangar_b = tw
        .world_mut()
        .create(ObjKind::Room, "Hangar B", GOD, None)
        .unwrap();
    bind(&mut tw, hangar_b, "sim.interior");
    ok(&mut tw, &format!("@spatial/space #{}=fixed:0", hangar_b.0));
    let t = ok(
        &mut tw,
        &format!("@vault/create bay_b=interior:#{}", hangar_b.0),
    );
    assert!(t.contains("Created vault #1"), "{t}");

    // Deposit fx.atlas into Vault #1 (V_B), then stand inside V_B and try to
    // re-deposit it into remote Vault #0 (V_A) — must be refused.
    ok(&mut tw, &format!("@vault/deposit #{}=1", fx.atlas.0));
    tw.world_mut().move_to(GOD, hangar_b).unwrap();
    let before_txs = journal_len(&tw);
    let err = refused(&mut tw, &format!("@vault/deposit #{}=0", fx.atlas.0));
    assert!(
        err.contains("already bound into the simulation"),
        "already-placed deposit must be refused: {err}"
    );
    assert_eq!(journal_len(&tw), before_txs);
    assert!(
        matches!(sim(&tw).parent_of(fx.atlas), SpatialParent::Interior { space, .. } if space == hangar_b),
        "atlas must stay in vault B"
    );

    // Direct-placed object on a domain cannot be yanked into V_A either.
    let direct_unit = tw
        .world_mut()
        .create(ObjKind::Thing, "Tank", GOD, Some(fx.field))
        .unwrap();
    bind(&mut tw, direct_unit, "sim.participant");
    ok(
        &mut tw,
        &format!("@spatial/place/domain #{}=0", direct_unit.0),
    );
    tw.world_mut().move_to(GOD, fx.field).unwrap();
    refused(&mut tw, &format!("@vault/deposit #{}=0", direct_unit.0));
    assert!(matches!(
        sim(&tw).parent_of(direct_unit),
        SpatialParent::Direct { .. }
    ));

    // Quarantined object cannot be deposited into V_A either.
    // Reach quarantine through the real space-destruction path (no fiat state write).
    let temp_bay = tw
        .world_mut()
        .create(ObjKind::Room, "TempBay", GOD, None)
        .unwrap();
    bind(&mut tw, temp_bay, "sim.interior");
    ok(&mut tw, &format!("@spatial/space #{}=fixed:0", temp_bay.0));
    let q_unit = tw
        .world_mut()
        .create(ObjKind::Thing, "Quarantined", GOD, Some(LIMBO))
        .unwrap();
    bind(&mut tw, q_unit, "sim.participant");
    ok(
        &mut tw,
        &format!("@spatial/place/interior #{}=#{}", q_unit.0, temp_bay.0),
    );
    tw.world_mut().destroy(temp_bay).expect("destroy temp bay");
    {
        let mut out = Outbox::new();
        let mut fxx = Effects::new();
        tw.engine_mut().emit_world_event(
            &WorldEvent::ObjDestroyed { obj: temp_bay },
            &mut out,
            &mut fxx,
        );
    }
    assert!(sim(&tw).is_quarantined(q_unit));
    let q_loc = loc(&tw, q_unit).unwrap_or(LIMBO);
    tw.world_mut().move_to(GOD, q_loc).unwrap();
    refused(&mut tw, &format!("@vault/deposit #{}=0", q_unit.0));
    assert!(sim(&tw).is_quarantined(q_unit));
}

/// **P2-MOD-23 (custody)**: `InteriorCustodyProvider::move_into` and `move_out`
/// refuse arena/field-deployed units.
#[test]
fn p2_mod_23_interior_custody_refuses_deployed_units() {
    let mut tw = boot();
    let fx = fixture(&mut tw);
    hangar_vault(&mut tw, &fx);

    mush_sim::deployed::mark(
        tw.world_mut(),
        fx.atlas,
        mush_sim::DeployedMark::new(77, "tacsim.arena.proving_ground"),
    );
    let err = refused(&mut tw, &format!("@vault/deposit #{}=0", fx.atlas.0));
    assert!(
        err.contains("deployed in arena match 77"),
        "deployed deposit must be refused: {err}"
    );
    assert!(!sim(&tw).is_placed(fx.atlas));

    mush_sim::deployed::clear(tw.world_mut(), fx.atlas);
    ok(&mut tw, &format!("@vault/deposit #{}=0", fx.atlas.0));

    mush_sim::deployed::mark(
        tw.world_mut(),
        fx.atlas,
        mush_sim::DeployedMark::field("site.alpha", 1),
    );
    let err = refused(&mut tw, &format!("@vault/withdraw #{}=0", fx.atlas.0));
    assert!(
        err.contains("in field 1 on site.alpha"),
        "deployed withdraw must be refused: {err}"
    );
    assert!(sim(&tw).is_placed(fx.atlas));
}

/// **P2-MOD-24**: `InteriorCustodyProvider::move_into` invokes `CarryAttempt`
/// hooks (such as `BayHold`/`CargoGate`/`InstalledGate`), and `move_out` skips
/// ONLY `PlacementGate` so `relay::InstalledGate` and all external hooks still run.
#[test]
fn p2_mod_24_interior_custody_enforces_carry_attempt_hooks_and_installed_gate() {
    use std::sync::atomic::{AtomicBool, Ordering};

    struct ExternalCarryVeto(Arc<AtomicBool>);
    impl mush_core::EventHook for ExternalCarryVeto {
        fn carry_attempt(
            &self,
            _ctx: &mush_core::HookReadContext<'_>,
            _mover: ObjId,
            _obj: ObjId,
            _dest: ObjId,
        ) -> Result<(), mush_proto::RichText> {
            if self.0.load(Ordering::SeqCst) {
                Err(mush_proto::RichText::tag(
                    "error",
                    "Blocked by external bay hold.",
                ))
            } else {
                Ok(())
            }
        }
    }
    struct ExternalGateModule(Arc<AtomicBool>);
    impl GameModule for ExternalGateModule {
        fn name(&self) -> &'static str {
            "ext_gate"
        }
        fn register(
            &mut self,
            reg: &mut mush_core::Registrar<'_>,
        ) -> Result<(), mush_core::ModError> {
            reg.hook(
                mush_core::HookPoint::CarryAttempt,
                Arc::new(ExternalCarryVeto(Arc::clone(&self.0))),
            );
            Ok(())
        }
    }

    let blocked = Arc::new(AtomicBool::new(true));
    let mut tw = TestWorld::with_modules(vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(SimulationModule::new()),
        Box::new(ExternalGateModule(Arc::clone(&blocked))),
        Box::new(mush_economy::EconomyModule::with_custody(vec![Arc::new(
            InteriorCustodyProvider,
        )])),
    ]);
    let fx = fixture(&mut tw);
    hangar_vault(&mut tw, &fx);

    // 1. External CarryAttempt hook vetoes move_into on an unplaced sim.participant.
    let err = refused(&mut tw, &format!("@vault/deposit #{}=0", fx.atlas.0));
    assert!(
        err.contains("Blocked by external bay hold."),
        "move_into must run CarryAttempt hooks: {err}"
    );
    assert!(!sim(&tw).is_placed(fx.atlas));

    // Allow deposit, then re-arm external veto and confirm move_out is vetoed.
    blocked.store(false, Ordering::SeqCst);
    ok(&mut tw, &format!("@vault/deposit #{}=0", fx.atlas.0));
    blocked.store(true, Ordering::SeqCst);
    let err = refused(&mut tw, &format!("@vault/withdraw #{}=0", fx.atlas.0));
    assert!(
        err.contains("Blocked by external bay hold."),
        "move_out must run external CarryAttempt hooks: {err}"
    );
    assert!(sim(&tw).is_placed(fx.atlas));

    // 2. Sim's own `relay::InstalledGate` (registered under module `NS = "sim"`)
    // must STILL veto both deposit AND withdraw (move_out skips ONLY PlacementGate).
    blocked.store(false, Ordering::SeqCst);
    let relay_half = tw
        .world_mut()
        .create(ObjKind::Thing, "Relay Half", GOD, Some(LIMBO))
        .unwrap();
    bind(&mut tw, relay_half, "sim.participant");
    bind(&mut tw, relay_half, mush_sim::relay::HALF_BEHAVIOR);
    let err = refused(&mut tw, &format!("@vault/deposit #{}=0", relay_half.0));
    assert!(
        err.contains("Installed relay hardware cannot be moved."),
        "move_into must enforce InstalledGate: {err}"
    );

    // Even if installed relay hardware is already placed inside the hangar space,
    // move_out must STILL enforce `relay::InstalledGate` (not skip all `NS` hooks).
    ok(
        &mut tw,
        &format!("@spatial/place/interior #{}=#{}", relay_half.0, fx.hangar.0),
    );
    let err = refused(&mut tw, &format!("@vault/withdraw #{}=0", relay_half.0));
    assert!(
        err.contains("Installed relay hardware cannot be moved."),
        "move_out must skip ONLY PlacementGate and enforce InstalledGate: {err}"
    );
    assert_eq!(loc(&tw, relay_half), Some(fx.hangar));
}

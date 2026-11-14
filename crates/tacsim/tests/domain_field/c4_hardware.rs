//! **Doc 44 unit MW.5, doc 17g C4 item 2: doc 17f §9's story 4, the hardware story, through the
//! launch's governed hardware, with a refit and a restart.**
//!
//! The world is the combined M8 world: the production `LaunchComposition` booted as the
//! bring-up boots it (`m8_world_field`: the plain world, the `Initialize` boot with its four
//! setup actions, the Registry's grants, the `Active` boot). Until doc 44 unit MW.23 no launch
//! world could hold a Class III core, so the hardware story ran only on the hardware flow's own
//! rig (`crates/tacsim/src/hardware/flow.rs`), which composes the hardware with no launch and
//! mints the entitlement by a test fixture. MW.23's stories end at the delivery. This one goes
//! on: the core is fitted, the server restarts, the core is read, and it is replaced.
//!
//! What the story types and what staff do. The pilot registers as an end user by their own verb
//! and pays for every core and every fitting from their own wallet. Staff do three things, each
//! the M8 world's standing practice and none of them the thing under test: they top the wallet
//! up; they title the pilot's two leased rigs to the pilot (no route in the game gives a player
//! title to a rig, and the refit asks for the titleholder); and they stand the pilot and a rig
//! in Basin's fitting shop and back in the bay (the shop is a facility of the launch's, not a
//! place on the map). The dent in the rig's plating is staff's too (`@unit/damage`), so that the
//! story does not need a fight to have something to keep.

use mush_core::ObjId;
use mush_economy::AccountId;
use mush_economy::core_integration::{CoreClass, DieSource, require_core_delivery};
use mush_industry::core_production::DieClass;
use mush_org::PrincipalId;
use serde_json::json;
use tacsim::definitions::core_replacement::{RIG_II, RIG_III};
use tacsim::launch::end_user::END_USER_DONE;
use tacsim::minds::registry_inspection::{HullClass, Provenance};

use crate::excursions::{economy, god, run};
use crate::m8_end_user::{credit, fund};
use crate::m8_world::{M8World, inspect, launch, m8_world_field, open_seal, recover, title_of};

/// The integration's and the refit's uniform refusals: what a requester is told whatever the
/// reason.
const REFIT_REFUSAL: &str = "Core refit is unavailable or changed.";

/// Where `thing` is.
fn place_of(world: &M8World, thing: ObjId) -> ObjId {
    world
        .rig
        .tw
        .world()
        .get(thing)
        .and_then(|o| o.location)
        .expect("it is somewhere")
}

/// Staff stand `thing` in `place`.
fn stand(world: &mut M8World, thing: ObjId, place: ObjId) {
    world
        .rig
        .tw
        .world_mut()
        .move_to(thing, place)
        .expect("stood there");
}

/// Staff release the lessor's claims on `tv` and title it to the pilot.
fn title_to_pilot(world: &mut M8World, tv: ObjId) {
    let pilot = world.pilot;
    let claims = economy(&world.rig.tw)
        .snapshot()
        .asset(tv)
        .map_or(0, |a| a.encumbrances.len());
    for k in (1..=claims).rev() {
        god(&mut world.rig, &format!("@asset/release #{}={k}", tv.0));
    }
    god(
        &mut world.rig,
        &format!("@asset/transfer #{}=#{}", tv.0, pilot.0),
    );
    assert_eq!(
        title_of(world, tv).title_owner,
        PrincipalId::Object(pilot),
        "the rig is the pilot's"
    );
}

/// The unit of `hull`.
fn unit(world: &M8World, hull: ObjId) -> mush_tactics::TacticalUnit {
    mush_tactics::module::unit_of(world.rig.tw.world(), hull)
        .expect("the rig's unit")
        .clone()
}

/// The core `hull` holds, as the arena reads it: its class and whether it is governed.
fn core_of(world: &M8World, hull: ObjId) -> Option<(tacsim::arena::core::CoreClass, bool)> {
    tacsim::arena::core::read_core(&unit(world, hull)).map(|hw| (hw.class, hw.governed))
}

/// **Where the kernel's refit record puts `core`** (`mush-core`'s `industry_refit_assets`
/// head): `pending`, `mounted`, `released` or `detached`. A core is admitted to a fitting with no
/// head, a `released` one or (doc 47 CO.4) a `detached` one, if it came out whole.
fn head_of(world: &M8World, core: ObjId) -> String {
    world
        .rows("industry_refit_assets")
        .into_iter()
        .find(|row| row.get("object") == Some(&json!(core)))
        .and_then(|row| {
            row.pointer("/position/state")
                .and_then(|state| state.as_str().map(str::to_owned))
        })
        .unwrap_or_else(|| panic!("the kernel holds a refit head for #{}", core.0))
}

/// **The refit's steps, each refusal returned**: the stock agent offers `core` (built under
/// `manufacture`) for `hull`, the pilot accepts paying from `payer`, the fitting's hours run and
/// the pilot installs. `Ok` when the core is in.
fn try_refit(
    world: &mut M8World,
    payer: AccountId,
    hull: ObjId,
    (manufacture, core): (mush_core::CommitToken, ObjId),
    class: DieClass,
) -> Result<(), String> {
    let pilot = world.pilot;
    let agent = world.deployment.hardware.agents[1].actor;
    let (offer, _) = world
        .try_perform(
            "industry",
            "offer_core_refit",
            agent,
            json!({"requester":pilot.0,"hull":hull.0,"incoming":core.0,
                "manufacture":manufacture.to_hex(),"class":class}),
        )
        .map_err(|refused| format!("offer: {refused}"))?;
    let (_, order) = world
        .try_perform(
            "industry",
            "accept_core_refit",
            pilot,
            json!({"offer":offer.to_hex(),"payer":payer.0.to_string()}),
        )
        .map_err(|refused| format!("accept: {refused}"))?;
    let id: mush_industry::workorder::WorkOrderId =
        serde_json::from_value(order["order"].clone()).expect("the work order");
    world.progress("advance_core_refit", [1, 2, 4][class.index()]);
    world
        .try_perform(
            "industry",
            "install_core_component",
            pilot,
            json!({"order":id.to_string()}),
        )
        .map(|_| ())
        .map_err(|refused| format!("install: {refused}"))
}

/// **Story 4** (doc 17f §9; this module's head). On the launch world, with nothing minted but
/// credits:
///
/// 1. **Permission at the transfer.** The pilot registers as an end user (`+enduser/apply go`).
/// 2. **Allotment, custody, integration, the titled core.** The importer's stock agent issues a
///    certified Class III die from the month's allotment and offers its integration; the pilot
///    pays the catalogue's price; the work runs its 96 hours at the chartered shop; the core is
///    delivered into the pilot's hands with a title born of that integration, certified, Class
///    III.
/// 3. **The refit.** The pilot's own coreless rig is fitted at the shop: its root becomes the
///    rig family's governed Class III and the pilot pays the fitting fee.
/// 4. **A restart, then the core read.** The server stops and boots over its store. The unit is
///    the same unit; the Registry's inspection reads that core, Class III, governed, certified
///    by the importer; and the rig's next launch seals over it, naming the core and its class.
/// 5. **The replacement.** With a dent in the rig's plating, a second core (Class II) is fitted
///    in the first one's place. The root is the family's Class II; the dent and every other
///    cell and component outside the core are as they were; the displaced Class III core is
///    back in the pilot's hands, still theirs, unencumbered.
/// 6. **A fitted core is not offered; a stored one is fitted again** (doc 47 CO.4, ruling
///    R-47-5, which reverses doc 17f §9's "a second install cannot reuse it"). The core fitted
///    in the first rig is in that hull and not in the pilot's hands, so the shop will not offer
///    it for the second rig, and nothing is charged. The displaced core is stored in the pilot's
///    hands (its kernel head `detached`), came out whole, and goes into the second rig: its root
///    is the rig family's Class III and the pilot pays the fitting fee.
/// 7. **A second restart** keeps both rigs as they stood.
#[test]
#[allow(clippy::too_many_lines)]
fn c4_story_4_a_class_three_core_is_fitted_read_after_a_restart_and_replaced() {
    let mut world = m8_world_field(("ahuna", "ahuna_port"), 2);
    let pilot = world.pilot;
    let (tv, spare) = (world.tvs[0], world.tvs[1]);
    let factory = world.deployment.hardware.factory;
    let importer = world.deployment.hardware.importer;
    let integrator = world.deployment.hardware.integrator.account;
    let certified = |grade| Provenance::Certified { importer, grade };
    fund(&mut world, pilot, 9_000_000);
    let (payer, _) = credit(&world, pilot);
    let (bay, deck) = (place_of(&world, tv), place_of(&world, pilot));
    for rig in [tv, spare] {
        title_to_pilot(&mut world, rig);
        assert_eq!(core_of(&world, rig), None, "a stock rig is coreless");
    }

    // 1. The registration, by the pilot's own verb.
    let done = run(&mut world.rig.tw, pilot, "+enduser/apply go");
    assert!(
        done.contains(END_USER_DONE.split("{fee}").next().expect("a head")),
        "{done}"
    );

    // 2. The allotment, the die, the integration and the titled core.
    let before = credit(&world, pilot).1;
    let (built, core) = world.integrate(pilot, payer, DieClass::III);
    assert_eq!(
        credit(&world, pilot).1,
        before - CoreClass::III.finished_minor(),
        "the core's price"
    );
    assert_eq!(place_of(&world, core), pilot, "delivered");
    let birth = economy(&world.rig.tw)
        .core_title_birth(core)
        .expect("the title's history")
        .expect("the core's birth");
    assert_eq!(
        (birth.class, birth.source),
        (CoreClass::III, DieSource::Certified)
    );
    assert_eq!(
        title_of(&world, core).title_owner,
        PrincipalId::Object(pilot),
        "the core is titled to its buyer"
    );
    assert!(require_core_delivery(world.rig.tw.world(), built).is_ok());

    // 3. The refit, at the chartered shop.
    for thing in [pilot, tv] {
        stand(&mut world, thing, factory);
    }
    let (wallet, shop) = (
        credit(&world, pilot).1,
        economy(&world.rig.tw)
            .snapshot()
            .account(integrator)
            .expect("the integrator's account")
            .posted_balance
            .minor(),
    );
    try_refit(&mut world, payer, tv, (built, core), DieClass::III)
        .unwrap_or_else(|refused| panic!("the fitting: {refused}"));
    let fitted = unit(&world, tv);
    assert_eq!(fitted.schema().key().id.as_str(), RIG_III);
    assert_eq!(
        core_of(&world, tv),
        Some((tacsim::arena::core::CoreClass::III, true)),
        "Class III, governed"
    );
    let fee = wallet - credit(&world, pilot).1;
    assert_eq!(
        fee,
        mush_economy::core_refit::FITTING_MINOR,
        "the fitting fee"
    );
    assert_eq!(
        economy(&world.rig.tw)
            .snapshot()
            .account(integrator)
            .expect("the integrator's account")
            .posted_balance
            .minor(),
        shop + fee,
        "paid to the shop"
    );
    stand(&mut world, tv, bay);
    stand(&mut world, pilot, deck);
    assert!(crate::excursions::in_bay(&world.rig.tw, tv, &world.row));

    // 4. A restart, then the core read: the unit, the Registry's inspection, the launch's seal.
    let mut world = world.reboot();
    assert_eq!(unit(&world, tv), fitted, "the same unit after the restart");
    assert!(require_core_delivery(world.rig.tw.world(), built).is_ok());
    let record = inspect(&mut world, tv);
    assert_eq!(record.core, Some(core));
    assert_eq!(record.class, HullClass::III);
    assert_eq!(record.governed, Some(true));
    assert_eq!(record.provenance, certified(CoreClass::III));
    let _ = launch(&mut world, "+tv/1 launch go");
    let facts = open_seal(&mut world, tv);
    assert!(facts.core.is_some(), "the seal names the core: {facts:?}");
    assert_eq!(
        (facts.core_class, facts.governed),
        (Some(tacsim::arena::core::CoreClass::III), true),
        "{facts:?}"
    );
    recover(&mut world);

    // 5. The replacement. A dent in the plating first, somewhere that is not the core.
    let pristine = unit(&world, tv);
    let (dented, value) = pristine
        .cells()
        .find_map(|(addr, cell)| {
            (matches!(addr, mush_tactics::CellAddr::Section { .. })
                && cell.current == cell.max
                && cell.current > cell.min + 1)
                .then(|| (addr.clone(), cell.current - 1))
        })
        .expect("a section cell with room for a dent");
    let dent = mush_tactics::UnitMutationPlan::new(
        vec![mush_tactics::UnitMutationOp::SetCell {
            addr: dented.clone(),
            value,
        }],
        Some(pristine.revision()),
        Some("a dent the refit must keep".into()),
    )
    .expect("a plan");
    god(
        &mut world.rig,
        &format!(
            "@unit/damage #{}={}",
            tv.0,
            serde_json::to_string(&dent).expect("the plan's text")
        ),
    );
    let before = unit(&world, tv);
    assert_eq!(before.cell(&dented).map(|cell| cell.current), Some(value));
    let (second_built, second) = world.integrate(pilot, payer, DieClass::II);
    for thing in [pilot, tv, spare] {
        stand(&mut world, thing, factory);
    }
    try_refit(&mut world, payer, tv, (second_built, second), DieClass::II)
        .unwrap_or_else(|refused| panic!("the replacement: {refused}"));
    let after = unit(&world, tv);
    assert_eq!(after.schema().key().id.as_str(), RIG_II);
    assert_eq!(
        core_of(&world, tv),
        Some((tacsim::arena::core::CoreClass::II, true))
    );
    assert_eq!(
        after.cell(&dented).map(|cell| cell.current),
        Some(value),
        "the dent is kept"
    );
    for (addr, cell) in before.cells() {
        if !matches!(addr, mush_tactics::CellAddr::Component { component, .. } if component.as_str() == "core")
        {
            assert_eq!(after.cell(addr), Some(cell), "{addr:?} is as it was");
        }
    }
    for (addr, _) in after.cells() {
        if !matches!(addr, mush_tactics::CellAddr::Component { component, .. } if component.as_str() == "core")
        {
            assert!(before.cell(addr).is_some(), "{addr:?} is no new cell");
        }
    }
    assert_eq!(
        after.components().count(),
        before.components().count(),
        "the refit adds and removes no component"
    );
    for (component, state) in before.components() {
        if component.as_str() != "core" {
            assert_eq!(
                after
                    .components()
                    .find(|(id, _)| *id == component)
                    .map(|(_, value)| value),
                Some(state),
                "{component:?} is as it was"
            );
        }
    }
    // The displaced core: back in the pilot's hands, still theirs, nothing claimed on it.
    assert_eq!(
        place_of(&world, core),
        pilot,
        "the displaced core is returned"
    );
    let displaced = title_of(&world, core);
    assert_eq!(displaced.title_owner, PrincipalId::Object(pilot));
    assert!(displaced.encumbrances.is_empty(), "{displaced:?}");

    // 6. A fitted core is not offered: it is in its hull, not in the pilot's hands. Nothing is
    // charged and nothing moves.
    let wallet = credit(&world, pilot).1;
    assert_eq!(head_of(&world, second), "mounted");
    assert_eq!(head_of(&world, core), "detached");
    let refused = try_refit(
        &mut world,
        payer,
        spare,
        (second_built, second),
        DieClass::II,
    )
    .expect_err("a fitted core is not offered for another hull");
    assert!(refused.starts_with("offer:"), "{refused}");
    assert!(refused.contains(REFIT_REFUSAL), "{refused}");
    assert_eq!(credit(&world, pilot).1, wallet, "nothing charged");
    assert_eq!(core_of(&world, spare), None);
    // The stored core is fitted again (R-47-5): into the second rig, for the fitting fee.
    try_refit(&mut world, payer, spare, (built, core), DieClass::III)
        .unwrap_or_else(|refused| panic!("the stored core's second fitting: {refused}"));
    assert_eq!(unit(&world, spare).schema().key().id.as_str(), RIG_III);
    assert_eq!(
        core_of(&world, spare),
        Some((tacsim::arena::core::CoreClass::III, true))
    );
    assert_eq!(head_of(&world, core), "mounted");
    assert_eq!(place_of(&world, core), spare);
    assert_eq!(
        credit(&world, pilot).1,
        wallet - mush_economy::core_refit::FITTING_MINOR
    );
    assert_eq!(
        core_of(&world, tv),
        Some((tacsim::arena::core::CoreClass::II, true))
    );
    for rig in [tv, spare] {
        stand(&mut world, rig, bay);
    }
    stand(&mut world, pilot, deck);

    // 7. A second restart keeps both.
    let (first, other) = (unit(&world, tv), unit(&world, spare));
    let mut world = world.reboot();
    assert_eq!(unit(&world, tv), first);
    assert_eq!(unit(&world, spare), other);
    for (rig, fitted, grade, class) in [
        (tv, second, CoreClass::II, HullClass::II),
        (spare, core, CoreClass::III, HullClass::III),
    ] {
        let record = inspect(&mut world, rig);
        assert_eq!(record.core, Some(fitted));
        assert_eq!(record.class, class);
        assert_eq!(record.governed, Some(true));
        assert_eq!(record.provenance, certified(grade));
    }
    crate::excursions::no_refused_writes(&world.rig.tw);
}

/// Staff mark the core in `hull` with `condition` (`@unit/damage`), as a fight would.
fn harm_core(world: &mut M8World, hull: ObjId, condition: &str) {
    let before = unit(world, hull);
    let plan = mush_tactics::UnitMutationPlan::new(
        vec![mush_tactics::UnitMutationOp::SetComponentCondition {
            component: mush_tactics::ComponentId::new("core").expect("the core's id"),
            condition: mush_tactics::ConditionId::new(condition).expect("a condition"),
        }],
        Some(before.revision()),
        Some("the core takes harm".into()),
    )
    .expect("a plan");
    god(
        &mut world.rig,
        &format!(
            "@unit/damage #{}={}",
            hull.0,
            serde_json::to_string(&plan).expect("the plan's text")
        ),
    );
}

/// The condition word the core in `hull` carries.
fn core_condition(world: &M8World, hull: ObjId) -> String {
    unit(world, hull)
        .component(&mush_tactics::ComponentId::new("core").expect("the core's id"))
        .expect("a core")
        .condition
        .as_str()
        .to_owned()
}

/// **Doc 47 CO.4, ruling R-47-5: a core a refit displaces, and that is not destroyed, is
/// stored and fitted again; a destroyed one is gone.** On the launch world, with nothing minted
/// but the pilot's starting credits (staff's top-up, as in story 4):
///
/// 1. **Stored.** The pilot's Class II core is fitted to the first rig and takes harm there
///    (`damaged`). A Class I core is fitted in its place; the Class II core comes out into the
///    pilot's hands, still titled to them and unencumbered: its kernel head reads `detached`.
/// 2. **Fitted again, as it came out.** The stored core is fitted to the second rig. Its root is
///    the rig family's Class II, and the core goes in `damaged`, not as new: a refit is not a
///    repair. The pilot pays the fitting fee and nothing else.
/// 3. **Stored again.** A fresh Class I core displaces it from the second rig, and it is back
///    in the pilot's hands, `detached`. (Its sale is doc 47 CO.13's: a governed core's title is
///    a managed title, which no generic contract may move.)
/// 4. **A destroyed core is gone.** The first rig's Class I core is destroyed, and a fresh
///    core displaces it. It comes out into the pilot's hands (the exchange returns what it
///    takes) but no fitting takes it: the shop's offer is refused, and nothing is charged.
#[test]
#[allow(clippy::too_many_lines)]
fn co4_a_displaced_core_is_stored_and_fitted_again() {
    let mut world = m8_world_field(("ahuna", "ahuna_port"), 2);
    let pilot = world.pilot;
    let (tv, spare) = (world.tvs[0], world.tvs[1]);
    let factory = world.deployment.hardware.factory;
    fund(&mut world, pilot, 9_000_000);
    let (payer, _) = credit(&world, pilot);
    for rig in [tv, spare] {
        title_to_pilot(&mut world, rig);
    }
    let done = run(&mut world.rig.tw, pilot, "+enduser/apply go");
    assert!(
        done.contains(END_USER_DONE.split("{fee}").next().expect("a head")),
        "{done}"
    );
    let (second_built, second) = world.integrate(pilot, payer, DieClass::II);
    let (first_built, first) = world.integrate(pilot, payer, DieClass::I);
    let (fresh_built, fresh) = world.integrate(pilot, payer, DieClass::I);
    let (last_built, last) = world.integrate(pilot, payer, DieClass::I);
    for thing in [pilot, tv, spare] {
        stand(&mut world, thing, factory);
    }

    // 1. Stored.
    try_refit(&mut world, payer, tv, (second_built, second), DieClass::II)
        .unwrap_or_else(|refused| panic!("the Class II fitting: {refused}"));
    harm_core(&mut world, tv, "damaged");
    try_refit(&mut world, payer, tv, (first_built, first), DieClass::I)
        .unwrap_or_else(|refused| panic!("the Class I fitting: {refused}"));
    assert_eq!(
        place_of(&world, second),
        pilot,
        "stored in the pilot's hands"
    );
    assert_eq!(head_of(&world, second), "detached");
    let stored = title_of(&world, second);
    assert_eq!(stored.title_owner, PrincipalId::Object(pilot));
    assert!(stored.encumbrances.is_empty(), "{stored:?}");

    // 2. Fitted again, carrying its condition.
    let wallet = credit(&world, pilot).1;
    try_refit(
        &mut world,
        payer,
        spare,
        (second_built, second),
        DieClass::II,
    )
    .unwrap_or_else(|refused| panic!("the stored core's fitting: {refused}"));
    assert_eq!(unit(&world, spare).schema().key().id.as_str(), RIG_II);
    assert_eq!(head_of(&world, second), "mounted");
    assert_eq!(place_of(&world, second), spare);
    assert_eq!(core_condition(&world, spare), "damaged", "not repaired");
    assert_eq!(
        credit(&world, pilot).1,
        wallet - mush_economy::core_refit::FITTING_MINOR,
        "the fitting fee and nothing else"
    );

    // 3. Stored again.
    try_refit(&mut world, payer, spare, (fresh_built, fresh), DieClass::I)
        .unwrap_or_else(|refused| panic!("the fresh core's fitting: {refused}"));
    assert_eq!(place_of(&world, second), pilot);
    assert_eq!(head_of(&world, second), "detached");
    assert_eq!(
        title_of(&world, second).title_owner,
        PrincipalId::Object(pilot),
        "still the pilot's"
    );

    // 4. A destroyed core is gone.
    harm_core(&mut world, tv, "destroyed");
    try_refit(&mut world, payer, tv, (last_built, last), DieClass::I)
        .unwrap_or_else(|refused| panic!("the last fitting: {refused}"));
    assert_eq!(place_of(&world, first), pilot);
    let wallet = credit(&world, pilot).1;
    let refused = try_refit(&mut world, payer, spare, (first_built, first), DieClass::I)
        .expect_err("a destroyed core is fitted into nothing");
    assert!(refused.starts_with("offer:"), "{refused}");
    assert!(refused.contains(REFIT_REFUSAL), "{refused}");
    assert_eq!(credit(&world, pilot).1, wallet, "nothing charged");
    assert_eq!(head_of(&world, first), "detached");
    crate::excursions::no_refused_writes(&world.rig.tw);
}

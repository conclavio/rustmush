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
use tacsim::definitions::core_replacement::{RIG_I, RIG_II, RIG_III};
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
/// head): `pending`, `mounted`, `released` or `detached`. A core is admitted to a fitting only
/// with no head or a `released` one.
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
/// 6. **A core is fitted once.** The core fitted in the first rig is in that hull and not in
///    the pilot's hands, so the shop will not offer it for the second rig. The displaced core
///    is in the pilot's hands and the shop offers it, but the kernel refuses the acceptance:
///    it admits a core to a fitting only if its head is absent or `released`
///    (`mush-core`'s `core_refit` heads), and a displaced core's reads `detached`. So it
///    stays the pilot's and stays out. Nothing is charged for either try. A third core, fresh
///    from the shop, goes into the second rig, so the second rig was never the reason.
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

    // 6. A core is fitted once. The fitted core is in its hull, not in the pilot's hands, and
    // the shop will not offer it. The displaced core is in the pilot's hands, the shop offers
    // it, and the kernel refuses the acceptance: its head reads `detached`. Nothing is charged
    // and nothing moves.
    let wallet = credit(&world, pilot).1;
    assert_eq!(head_of(&world, second), "mounted");
    assert_eq!(head_of(&world, core), "detached");
    for (name, again, class, stage) in [
        (
            "the fitted core",
            (second_built, second),
            DieClass::II,
            "offer:",
        ),
        (
            "the displaced core",
            (built, core),
            DieClass::III,
            "accept:",
        ),
    ] {
        let refused = try_refit(&mut world, payer, spare, again, class)
            .expect_err("a core that has been mounted is not fitted again");
        assert!(refused.starts_with(stage), "{name}: {refused}");
        assert!(refused.contains(REFIT_REFUSAL), "{name}: {refused}");
        assert_eq!(credit(&world, pilot).1, wallet, "{name}: nothing charged");
        assert_eq!(core_of(&world, spare), None, "{name}");
    }
    assert_eq!(
        core_of(&world, tv),
        Some((tacsim::arena::core::CoreClass::II, true))
    );
    assert_eq!(place_of(&world, core), pilot);
    assert_eq!(
        title_of(&world, core).title_owner,
        PrincipalId::Object(pilot),
        "the displaced core is still the pilot's"
    );
    // A core fresh from the shop goes into the second rig: the rig was never the reason.
    let (third_built, third) = world.integrate(pilot, payer, DieClass::I);
    for thing in [pilot, spare] {
        stand(&mut world, thing, factory);
    }
    let wallet = credit(&world, pilot).1;
    try_refit(&mut world, payer, spare, (third_built, third), DieClass::I)
        .unwrap_or_else(|refused| panic!("a fresh core's fitting: {refused}"));
    assert_eq!(unit(&world, spare).schema().key().id.as_str(), RIG_I);
    assert_eq!(
        credit(&world, pilot).1,
        wallet - mush_economy::core_refit::FITTING_MINOR
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
        (spare, third, CoreClass::I, HullClass::I),
    ] {
        let record = inspect(&mut world, rig);
        assert_eq!(record.core, Some(fitted));
        assert_eq!(record.class, class);
        assert_eq!(record.governed, Some(true));
        assert_eq!(record.provenance, certified(grade));
    }
    crate::excursions::no_refused_writes(&world.rig.tw);
}

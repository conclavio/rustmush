//! **Doc 41 cut RS.1 — the tender (story a, the ground half)** (Q-41-1..4, Q-41-27, Q-41-29,
//! Q-38-16, Q-36-35/36), over the composed world (`driving`'s keyed rig), the arena's egress
//! pumped by hand as `field_gc4.rs` pumps it, on both grounds of `grounds_rig` (Ahuna Flank at
//! 256, Urvara Reach at 512).
//!
//! The captain leases Occator's escort and its tender (`+tv/lease escort`, `+tv/lease tender`),
//! sails to the site, and on the pad stocks the tender from the Dray-P's hold (`+tv/2 stores
//! load`), opens its bale (`+tv/2 stores open`) and launches both. The escort stands beside the
//! tender with a jammed cannon, a damaged hull and a short magazine. The tender clears the jam
//! (`+tv/2 unjam`), patches the hull (`+tv/2 stabilize`) and refills the magazine (`+tv/2
//! rearm`); a stranger's mark opens a field over the pair during each act, which pauses it, and
//! the act resumes when the field settles and lands. The bale and the stores are drawn as the
//! tariff says. Then the laden tow: a second party's prospector, its hopper holding 7 t of ore,
//! is refused by its own 6 t rig on her 13 t laden (a dry 6 t would pass the 2× cap), and the
//! tender's 3× coupling takes her.
//!
//! **What is a fixture.** The escort's damage (the jam, the hull's internals, the magazine) and
//! the stranger's ore and power, by staff hand, standing in for a fight and a dig (GC.3's
//! precedent); the hold's goods, seeded. Every act after is the production command path.

use mush_core::ObjId;
use tacsim::arena::ledger::LedgerEvent;
use tacsim::surface::tow::TowStage;

use crate::driving_rig::{
    Rig, act_go, command, finish, ic_now, land, on_pad_as_at, on_pad_with_at, ramp_at, run,
    seed_hold, seed_pool, staff_hand, stocked_tuned, to_cell_on, to_ic, warm,
};
use crate::field_host_rig::{
    armed_on, egress, seat, second, set_section, site_record_at, task, tune, until,
};
use crate::grounds_rig::{AHUNA_FLANK, Ground, URVARA_REACH};

/// The tender's acts, `{n}` its slot.
fn act_of(rig: &mut Rig, ground: &Ground, tv: ObjId) -> Option<tacsim::surface::tow::TowAct> {
    site_record_at(rig, ground.entry).and_then(|record| record.tows.act_of(tv).cloned())
}

/// Whether `tv`'s tend act is paused.
fn paused(rig: &mut Rig, ground: &Ground, tv: ObjId) -> bool {
    matches!(
        act_of(rig, ground, tv).map(|act| act.stage),
        Some(TowStage::Tend {
            paused: Some(_),
            ..
        })
    )
}

/// Staff money for the two leases' deposits (`bay_gc8`'s fixture).
fn mint(rig: &mut Rig, pilot: ObjId) {
    let treasury = crate::driving_rig::port_map(&rig.tw, "ceres_port")
        .treasury
        .expect("a treasury");
    let account = tacsim::port::credit::own_account(rig.tw.world(), pilot)
        .map(|a| a.id.0)
        .expect("the pilot's account");
    let _ = run(
        &mut rig.tw,
        mush_core::GOD,
        &format!("@econ/mint {treasury}={account} 10000000"),
    );
}

/// One staff mutation on `unit`.
fn mutate(rig: &mut Rig, unit: ObjId, ops: Vec<mush_tactics::UnitMutationOp>) {
    staff_hand(rig, |ctx| {
        let plan = mush_tactics::UnitMutationPlan::new(
            ops,
            None,
            Some(smol_str::SmolStr::new_static("test:rs1-damage")),
        )
        .expect("a plan");
        mush_tactics::module::apply_plan(ctx, unit, &plan).expect("applied");
    });
}

/// `unit`'s cell value at `addr`.
fn cell(rig: &Rig, unit: ObjId, addr: &mush_tactics::CellAddr) -> i64 {
    mush_tactics::module::unit_of(rig.tw.world(), unit)
        .and_then(|cells| cells.cell(addr).map(|c| c.current))
        .expect("the cell")
}

/// The escort's cannon's condition.
fn cannon(rig: &Rig, escort: ObjId) -> String {
    let unit = mush_tactics::module::unit_of(rig.tw.world(), escort).expect("the escort");
    unit.components()
        .find(|(id, _)| id.as_str().rsplit('.').next() == Some("cannon"))
        .map(|(_, c)| c.condition.as_str().to_owned())
        .expect("a cannon")
}

/// **Start a tend act with `line`, open a field over the pair while it runs, see it paused, let
/// the field settle, see it resume, and land it** — answers what the tender was told.
#[allow(clippy::too_many_arguments)]
async fn tend_through_a_field(
    rig: &mut Rig,
    ground: &Ground,
    (rx, lines): (
        &mut tokio::sync::mpsc::Receiver<crate::field_host_rig::ArenaOut>,
        &mut Vec<LedgerEvent>,
    ),
    (a, b, tender): (ObjId, ObjId, ObjId),
    line: &str,
) {
    let (out, queued) = command(rig, a, &format!("{line} go"));
    assert_eq!(queued.len(), 1, "one tend start: {out}");
    assert_eq!(
        queued[0].action.key().id.as_str(),
        tacsim::surface::tow::TOW_ACTION
    );
    land(rig, &queued[0]).expect("the tend start lands");
    let act = act_of(rig, ground, tender).expect("the tend act is on the record");
    assert!(act.due_ic > act.started_ic, "{line}: the work takes time");
    // The tender tends or drives, never both.
    let busy = run(&mut rig.tw, a, "+tv/2 scan");
    assert!(busy.contains("busy with its tow"), "{busy}");

    // The stranger's mark opens a field over the pair: the act pauses.
    let (out, queued) = command(rig, b, "+field/mark E=hostile");
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    land(rig, &queued[0]).expect("the promotion lands");
    assert!(
        until(rig, rx, lines, 20, |rig, _| task(rig).is_some()).await,
        "{line}: the field opens"
    );
    assert!(
        until(rig, rx, lines, 20, |rig, _| paused(rig, ground, tender)).await,
        "{line}: the field pauses the act"
    );
    assert!(
        until(rig, rx, lines, 180, |rig, _| task(rig).is_none()
            && site_record_at(rig, ground.entry)
                .is_some_and(|record| record.open_field().is_none()))
        .await,
        "{line}: the field settles"
    );
    // The act resumes on open ground, and lands at its due instant.
    for _ in 0..5 {
        if !paused(rig, ground, tender) {
            break;
        }
        let _ = second(rig);
    }
    let act = act_of(rig, ground, tender).expect("the act survives the field");
    assert!(!paused(rig, ground, tender), "{line}: the act resumes");
    let _ = to_ic(rig, act.due_ic);
    for _ in 0..5 {
        if act_of(rig, ground, tender).is_none() {
            break;
        }
        let errors = second(rig);
        assert!(
            errors
                .iter()
                .all(|e| !e.contains(tacsim::surface::tow::TOW_ACTION)),
            "{line}: a tend landing refused: {errors:?}"
        );
    }
    assert!(
        act_of(rig, ground, tender).is_none(),
        "{line}: the act has landed"
    );
}

/// **RS.1's story on `ground`.**
#[allow(clippy::too_many_lines)] // one story, in order
async fn tender_on(ground: &Ground) {
    let mut rig = stocked_tuned(|cfg| {
        tune(cfg);
        cfg.field.idle_demote_s = 15;
        cfg.recovery.inspect_s = 30;
        cfg.recovery.attach_s = 30;
    });
    // The captain's party: the escort (rig 1) and the tender (rig 2) in a Dray-P's bay.
    let (a, row, tvs) = on_pad_with_at(
        &mut rig,
        ("Pilot", 1, "Salt Moth"),
        ground.site(),
        |rig, pilot| {
            mint(rig, pilot);
            crate::driving_rig::at_counter(rig, pilot);
            let escort = run(&mut rig.tw, pilot, "+tv/lease escort go");
            assert!(
                escort.contains("Occator Water & Propellant leases you "),
                "{escort}"
            );
            let tender = run(&mut rig.tw, pilot, "+tv/lease tender go");
            assert!(
                tender.contains("Occator Water & Propellant leases you "),
                "{tender}"
            );
        },
    );
    let (escort, tender) = (tvs[0], tvs[1]);
    assert_eq!(
        tacsim::surface::tender::tender_def(rig.tw.world(), tender).map(|d| d.reach_cm),
        Some(1_500),
        "the tender's definition carries the role"
    );
    assert!(tacsim::surface::tender::tender_def(rig.tw.world(), escort).is_none());

    // The stores from the hold, on the pad, in the bay; the bale opened before launch.
    seed_hold(&mut rig, &row, "structural_alloys", 2, 9_501);
    seed_hold(&mut rig, &row, "industrial_parts", 2, 9_502);
    seed_hold(&mut rig, &row, "ammo_he_105", 30, 9_503);
    let refused = run(&mut rig.tw, a, "+tv/2 stores open");
    assert!(refused.contains("no whole tonne"), "{refused}");
    for line in [
        "+tv/2 stores load 1 structural_alloys go",
        "+tv/2 stores load 1 industrial_parts go",
        "+tv/2 stores open go",
        "+tv/2 stores load 20 ammo_he_105 go",
    ] {
        let (out, queued) = command(&mut rig, a, line);
        assert_eq!(queued.len(), 1, "{line}: one stores op: {out}");
        land(&mut rig, &queued[0]).unwrap_or_else(|e| panic!("{line}: {e}"));
    }
    let shown = run(&mut rig.tw, a, "+tv/2 stores");
    assert!(
        shown.contains("20 ammo_he_105") && shown.contains("1000 kg of structural alloys"),
        "{shown}"
    );
    let noroom = run(&mut rig.tw, a, "+tv/1 stores");
    assert!(noroom.contains("no tender"), "{noroom}");

    // Both launch; they stand side by side beside the ramp.
    let _ = act_go(&mut rig, a, "+tv/1 launch go");
    let _ = finish(&mut rig, escort, a);
    let _ = act_go(&mut rig, a, "+tv/2 launch go");
    let _ = finish(&mut rig, tender, a);
    let (rx, ry, _) = ramp_at(&rig.tw, ground.entry);
    let ramp = (rx.div_euclid(1_000), ry.div_euclid(1_000));
    to_cell_on(&mut rig, ground.entry, escort, (ramp.0 + 4, ramp.1));
    to_cell_on(&mut rig, ground.entry, tender, (ramp.0 + 5, ramp.1));
    // The fixture: the escort's cannon jammed, its hull down 40 points, its magazine at 10.
    let cannon_id = mush_tactics::unit::ComponentId::new("cannon").expect("an id");
    mutate(
        &mut rig,
        escort,
        vec![mush_tactics::UnitMutationOp::SetComponentCondition {
            component: cannon_id.clone(),
            condition: mush_tactics::ConditionId::new("jammed").expect("a condition"),
        }],
    );
    let internals = mush_tactics::CellAddr::Section {
        section: mush_tactics::SectionId::new("hull").expect("an id"),
        cell: mush_tactics::CellId::new(tacsim::definitions::unit::INTERNALS_CELL).expect("an id"),
    };
    let full = cell(&rig, escort, &internals);
    set_section(&mut rig, escort, "hull", Some("damaged"), Some(full - 40));
    let ammo = mush_tactics::CellAddr::Component {
        component: cannon_id,
        cell: mush_tactics::CellId::new(tacsim::definitions::unit::AMMO_CELL).expect("an id"),
    };
    let magazine = mush_tactics::module::unit_of(rig.tw.world(), escort)
        .and_then(|cells| cells.cell(&ammo).map(|c| c.max))
        .expect("a magazine");
    mutate(
        &mut rig,
        escort,
        vec![mush_tactics::UnitMutationOp::SetCell {
            addr: ammo.clone(),
            value: 10,
        }],
    );
    assert_eq!(cannon(&rig, escort), "jammed");

    // The stranger, whose mark opens each field; and its two prospectors for the laden tow.
    let (b, _, others) = on_pad_as_at(&mut rig, ("Second", 2, "Red Moth"), ground.site(), 2);
    let hound = armed_on(
        &mut rig,
        ground.entry,
        "Red Hound",
        (ramp.0 - 8, ramp.1),
        90_000,
    );
    seat(&mut rig, hound, b);
    seat(&mut rig, tender, a);
    // The laden casualty (Q-41-29): the stranger's prospector, 7 t of ore in her hopper and no
    // power, west of the captain's pair beside its other rig; the fields below index her a
    // casualty.
    let (casualty, tug) = (others[0], others[1]);
    let _ = act_go(&mut rig, b, "+tv/1 launch go");
    let _ = finish(&mut rig, casualty, b);
    let _ = act_go(&mut rig, b, "+tv/2 launch go");
    let _ = finish(&mut rig, tug, b);
    to_cell_on(&mut rig, ground.entry, casualty, (ramp.0 + 2, ramp.1));
    to_cell_on(&mut rig, ground.entry, tug, (ramp.0 + 1, ramp.1));
    let hopper = tacsim::port::vehicle::hopper_of(rig.tw.world(), casualty).expect("a hopper");
    seed_pool(
        &mut rig,
        hopper,
        "metal_ore",
        7,
        mush_inventory::LotId(9_504),
    );
    mutate(
        &mut rig,
        casualty,
        vec![mush_tactics::UnitMutationOp::SetComponentCondition {
            component: mush_tactics::unit::ComponentId::new("power").expect("an id"),
            condition: mush_tactics::ConditionId::new("destroyed").expect("a condition"),
        }],
    );
    warm(&mut rig);
    let mut rx = egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();

    // The previews (Q-41-3): the tariff's time at three times, the draw named.
    let preview = run(&mut rig.tw, a, &format!("+tv/2 unjam #{}", escort.0));
    assert!(
        preview.contains("clear the jam")
            && preview.contains("10 kg of yard fittings")
            && preview.contains("about 360 IC minutes"),
        "{preview}"
    );
    let preview = run(
        &mut rig.tw,
        a,
        &format!("+tv/2 stabilize #{}=hull", escort.0),
    );
    assert!(
        preview.contains("patch 40 points of hull")
            && preview.contains("400 kg of yard fittings")
            && preview.contains("about 72 IC minutes"),
        "{preview}"
    );
    let far = run(&mut rig.tw, b, &format!("+tv/1 unjam #{}", escort.0));
    assert!(far.contains("no tender"), "{far}");

    // (1) The jam cleared, through a field.
    tend_through_a_field(
        &mut rig,
        ground,
        (&mut rx, &mut lines),
        (a, b, tender),
        &format!("+tv/2 unjam #{}", escort.0),
    )
    .await;
    assert_ne!(cannon(&rig, escort), "jammed", "the jam is cleared");
    // (2) The hull patched, through a field.
    tend_through_a_field(
        &mut rig,
        ground,
        (&mut rx, &mut lines),
        (a, b, tender),
        &format!("+tv/2 stabilize #{}=hull all", escort.0),
    )
    .await;
    assert_eq!(cell(&rig, escort, &internals), full, "the hull is whole");
    // (3) The magazine refilled from the stores, through a field.
    tend_through_a_field(
        &mut rig,
        ground,
        (&mut rx, &mut lines),
        (a, b, tender),
        &format!("+tv/2 rearm #{}", escort.0),
    )
    .await;
    let short = magazine - 10;
    assert_eq!(
        cell(&rig, escort, &ammo),
        10 + short.min(20),
        "the magazine takes what the stores held, up to its size"
    );
    // The draw: 10 + 400 kg of fittings out of the bale's tonne; the rounds out of the stores.
    let bale = crate::driving_rig::record_of(&mut rig.tw, tender)
        .and_then(|record| record.bale)
        .expect("the bale");
    assert_eq!(bale.fittings_kg, 1_000 - 10 - 400, "the bale's fittings");
    assert_eq!(
        bale.alloys_kg, 1_000,
        "no alloys until IN.8's per-point figures"
    );
    let shown = run(&mut rig.tw, a, "+tv/2 stores");
    let left = 20 - u64::try_from(short.min(20)).expect("rounds");
    assert!(
        if left == 0 {
            shown.contains("hold nothing")
        } else {
            shown.contains(&format!("{left} ammo_he_105"))
        },
        "{shown}"
    );

    // The laden tow (Q-41-29, Q-41-27): the stranger's prospector, indexed a casualty by the
    // fields above.
    // The fields indexed her a casualty where she lies.
    assert!(
        site_record_at(&mut rig, ground.entry)
            .is_some_and(|record| record.units.iter().any(|left| left.unit == casualty)),
        "the casualty is on the site's index"
    );
    let _ = second(&mut rig);
    let refused = run(&mut rig.tw, b, &format!("+tv/2 tow #{}", casualty.0));
    assert!(
        refused.contains("at 13 t laden it is more than the 12 t a 6 t tug can tow"),
        "her laden mass is over the prospector's 2x, where her dry 6 t is under it: {refused};          index {:?}, casualty at {:?}, tug at {:?}",
        site_record_at(&mut rig, ground.entry).map(|r| r.units),
        mush_sim::pose_of(rig.tw.world(), casualty),
        mush_sim::pose_of(rig.tw.world(), tug)
    );
    // The tender drives up beside her (a staff hand standing in for its DRIVE).
    to_cell_on(&mut rig, ground.entry, tender, (ramp.0 + 3, ramp.1 + 1));
    let taken = run(&mut rig.tw, a, &format!("+tv/2 tow #{}", casualty.0));
    assert!(
        taken.contains("13 t") && taken.contains("In tow it moves at"),
        "the tender's 3x coupling takes her: {taken}"
    );
    let _ = ic_now(&rig.tw);
}

/// **RS.1 at 256**: Ahuna Flank.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rs1_ahuna_the_tender_works_through_fields_and_the_tow_counts_her_laden() {
    tender_on(&AHUNA_FLANK).await;
}

/// **RS.1 at 512**: Urvara Reach.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rs1_urvara_the_tender_works_through_fields_and_the_tow_counts_her_laden() {
    tender_on(&URVARA_REACH).await;
}

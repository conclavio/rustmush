//! **Doc 41 cut RS.4 — the vacated hull's return (story b, extended)** (Q-41-18, Q-41-19,
//! Q-38-19; round-2 item 41-15), over the composed world (`driving`'s keyed rig) on Urvara Reach
//! (512) and on Ahuna Flank (256):
//!
//! - **the owner** (T), whose carrier stands on the pad with an empty bay, and whose two rigs `a`
//!   and `d` lie beside it on L's blocks;
//! - **the opportunist** (O), whose rig tows both into O's carrier with nobody's authority;
//! - **the landholder** (L), who holds the Registry's claims on those blocks.
//!
//! O files on both and T's `recovery` contests vacate both: each is held for T on her bay record,
//! and nothing O does moves her. On the pad, T aboard T's own carrier takes `a` out of O's bay
//! bay to bay (`+bay/stow #a=<T's carrier>`). O sails home to Ceres with `d`: at the docking the
//! port berths her in the workshop "held for" T, outside the stand count, and O's slot frees.
//! T's carrier docks at Ceres later and T stows her from the workshop floor.
//!
//! **What is a fixture.** As RS.3's story: the rigs' power is destroyed and they are put on the
//! site's index by staff hand, moved to T by `@asset/transfer`, and L's block claims seeded as
//! typed records; T is put in the workshop room by hand (the walk there is not the subject).

use mush_core::{GOD, ObjId};
use mush_org::PrincipalId;
use tacsim::surface::act::{Where, where_is};
use tacsim::surface::salvage::{ClaimSeam, SalvageCustody};
use tacsim::surface::site_record::SiteRecord;

use crate::driving_rig::{
    Rig, cid, command, file_and_launch, launched_at, on_pad_as_at, pilot, port_map, ramp_at, run,
    seed_typed, staff_hand, station_room, stocked_tuned, teleport, to_cell_on, to_ic,
};
use crate::field_host_rig::{second, site_record_at, tune};
use crate::site_record_rig::{staff, staff_member};

/// The compressed contest window, real seconds.
const WINDOW: i64 = 600;

/// Short tow stages and a ten-minute contest window, on the field host's lab tempo.
fn quick(cfg: &mut tacsim::TacsimConfig) {
    tune(cfg);
    cfg.recovery.inspect_s = 30;
    cfg.recovery.attach_s = 30;
    cfg.recovery.load_s = 60;
    cfg.recovery.claim_window_s = WINDOW;
}

fn record(rig: &mut Rig, entry: &str) -> SiteRecord {
    site_record_at(rig, entry).expect("the site's record")
}

fn custody(rig: &mut Rig, entry: &str, hull: ObjId) -> Option<SalvageCustody> {
    record(rig, entry)
        .custody
        .into_iter()
        .find(|row| row.hull == hull)
}

/// `line` as `who`, every submission it queued landed; the reply.
fn go(rig: &mut Rig, who: ObjId, line: &str) -> String {
    let (out, queued) = command(rig, who, line);
    assert!(!queued.is_empty(), "{line}: a submission: {out}");
    for submit in &queued {
        crate::driving_rig::land(rig, submit).unwrap_or_else(|e| panic!("{line}: {e}"));
    }
    out
}

/// Seconds until `done` holds, at most `limit`, every tow landing accepted.
fn seconds_until(rig: &mut Rig, limit: u32, mut done: impl FnMut(&mut Rig) -> bool) -> bool {
    for _ in 0..limit {
        if done(rig) {
            return true;
        }
        let errors: Vec<String> = second(rig)
            .into_iter()
            .filter(|error| error.contains(tacsim::surface::tow::TOW_ACTION))
            .collect();
        assert!(errors.is_empty(), "a tow landing refused: {errors:?}");
    }
    done(rig)
}

/// **Destroy `unit`'s power component** (GC.4's fixture): a casualty.
fn powerless(rig: &mut Rig, unit: ObjId) {
    staff_hand(rig, |ctx| {
        let plan = mush_tactics::UnitMutationPlan::new(
            vec![mush_tactics::UnitMutationOp::SetComponentCondition {
                component: mush_tactics::unit::ComponentId::new("power").expect("an id"),
                condition: mush_tactics::ConditionId::new("destroyed").expect("a condition"),
            }],
            None,
            Some(smol_str::SmolStr::new_static("test:rs4-powerless")),
        )
        .expect("a plan");
        mush_tactics::module::apply_plan(ctx, unit, &plan).expect("applied");
    });
}

/// The block letter of `cell` on a site `side` cells a side.
fn letter_of(side: u16, cell: (i64, i64)) -> char {
    tacsim::field_ops::block_letter_at_site_pose(
        mush_tactics::field_ops::FieldPose {
            site: ObjId(1),
            x_cm: cell.0 * 1_000 + 500,
            y_cm: cell.1 * 1_000 + 500,
        },
        tacsim::surface::address::Scheme::of_site(side),
    )
    .expect("a block")
}

/// L's claim on `letter` of `entry`, seeded (GC.6's fixture).
fn seed_block(rig: &mut Rig, entry: &str, letter: char, l: ObjId) {
    let place = port_map(&rig.tw, entry)
        .site
        .and_then(|site| site.blocks.get(letter.to_string().as_str()).copied())
        .map(ObjId)
        .expect("the block's place");
    let now = rig.tw.world().world_ic_now().at;
    let mut tenure = tacsim::surface::claims::BlockTenure::new(cid(entry), letter, place);
    tenure.claim = Some(tacsim::surface::claims::BlockClaim {
        holder: PrincipalId::Object(l),
        account: rig
            .tw
            .world()
            .module_state::<mush_sim::AccountOracle>()
            .and_then(|oracle| oracle.account_of(l)),
        epoch: 1,
        opened_ic: None,
        registered_ic: now,
        term_start_ic: now,
        term_end_ic: now + 365 * 86_400,
        worked_at_term_start: 0,
        discovered_ic: None,
        declared: None,
        fee: None,
        refund: None,
        lapsed_ic: None,
    });
    let handles = rig
        .tw
        .world()
        .module_state::<tacsim::port::PortHandles>()
        .cloned()
        .expect("the port's handles");
    seed_typed(
        rig,
        &handles.families.claims,
        &tacsim::surface::claims::BlockKey {
            site: cid(entry),
            letter,
        },
        tenure,
    );
}

/// O's rig tows `casualty` from beside it and loads it into O's carrier at the ramp.
fn recover(rig: &mut Rig, entry: &str, (o, tug): (ObjId, ObjId), casualty: ObjId, at: (i64, i64)) {
    to_cell_on(rig, entry, tug, (at.0 + 1, at.1));
    let _ = second(rig);
    let out = go(rig, o, &format!("+tv/1 tow #{} go", casualty.0));
    assert!(
        seconds_until(rig, 120, |rig| record(rig, entry)
            .tows
            .towing(tug)
            .map(|op| op.casualty())
            == Some(casualty)),
        "the tow attaches: {out}"
    );
    let ramp = {
        let (x, y, _) = ramp_at(&rig.tw, entry);
        (x.div_euclid(1_000), y.div_euclid(1_000))
    };
    to_cell_on(rig, entry, tug, ramp);
    let _ = second(rig);
    let out = go(rig, o, "+tv/1 load go");
    assert!(
        seconds_until(rig, 120, |rig| custody(rig, entry, casualty).is_some()
            && record(rig, entry).tows.act_of(tug).is_none()),
        "the load lands in custody: {out}"
    );
}

/// Whom `hull` is held for.
fn held_for(rig: &Rig, hull: ObjId) -> Option<PrincipalId> {
    tacsim::port::bay_work::held_for(rig.tw.world(), hull)
}

/// **The story at one site**: `(venue, entry, side)`.
#[allow(clippy::too_many_lines)]
fn story(venue: &str, entry: &str, side: u16) {
    let mut rig = stocked_tuned(quick);
    let (t, t_row, _) = on_pad_as_at(&mut rig, ("Pilot", 1, "Salt Moth"), (venue, entry), 0);
    let t_hull = ObjId(t_row.hull);
    let (o, o_row, o_tvs) = on_pad_as_at(&mut rig, ("Second", 2, "Red Moth"), (venue, entry), 1);
    let o_hull = ObjId(o_row.hull);
    let tug = o_tvs[0];
    let l = pilot(&mut rig, "Lander", 3);
    // Two spare rigs of Occator's pool, titled to T.
    let schema = mush_tactics::module::unit_of(rig.tw.world(), tug)
        .expect("the tug's unit")
        .schema()
        .clone();
    let spares: Vec<ObjId> = rig
        .tw
        .world()
        .module_state::<mush_tactics::UnitStore>()
        .expect("the units")
        .units
        .iter()
        .filter(|(id, unit)| **id != tug && unit.schema() == &schema)
        .map(|(id, _)| *id)
        .filter(|id| {
            crate::driving_rig::store(&rig.tw)
                .tv_lease_on(*id)
                .is_none()
        })
        .take(2)
        .collect();
    let [a, d] = spares.as_slice() else {
        panic!("two spare rigs in Occator's pool: {spares:?}");
    };
    let (a, d) = (*a, *d);
    for hull in [a, d] {
        let moved = run(
            &mut rig.tw,
            GOD,
            &format!("@asset/transfer #{}=#{}", hull.0, t.0),
        );
        assert!(!moved.contains("Permission denied"), "{moved}");
    }
    let (rx, ry) = {
        let (x, y, _) = ramp_at(&rig.tw, entry);
        (x.div_euclid(1_000), y.div_euclid(1_000))
    };
    let (a_cell, d_cell) = ((rx + 3, ry - 2), (rx - 3, ry - 2));
    let s = staff_member(&mut rig);
    for (hull, (x, y)) in [(a, a_cell), (d, d_cell)] {
        to_cell_on(&mut rig, entry, hull, (x, y));
        powerless(&mut rig, hull);
        staff(
            &mut rig,
            s,
            &format!("@tacsim/site {entry} leave #{} casualty {x},{y}", hull.0),
        );
        seed_block(&mut rig, entry, letter_of(side, (x, y)), l);
    }

    // 1. O recovers and files on both; T's sensor record vacates both, and each is held for T.
    launched_at(&mut rig, entry, o, tug);
    recover(&mut rig, entry, (o, tug), a, a_cell);
    recover(&mut rig, entry, (o, tug), d, d_cell);
    for hull in [a, d] {
        let out = go(&mut rig, o, &format!("+salvage/claim #{} go", hull.0));
        assert!(out.contains("You file a salvage claim"), "{out}");
        assert!(matches!(
            custody(&mut rig, entry, hull).map(|row| row.claim),
            Some(ClaimSeam::Filed { .. })
        ));
        let _ = go(
            &mut rig,
            t,
            &format!("+salvage/contest #{}=recovery go", hull.0),
        );
    }
    assert!(
        seconds_until(&mut rig, 10, |rig| custody(rig, entry, a).is_none()
            && custody(rig, entry, d).is_none()),
        "both vacating contests settle"
    );
    for hull in [a, d] {
        assert_eq!(held_for(&rig, hull), Some(PrincipalId::Object(t)));
        assert_eq!(where_is(rig.tw.world(), hull), Where::Bay { hull: o_hull });
    }
    let shown = run(&mut rig.tw, t, &format!("+bay #{}", a.0));
    assert!(shown.contains("is held for Pilot"), "{shown}");

    // 2. Nothing O does moves her; T aboard T's carrier on the same pad takes `a` bay to bay.
    let refused = run(&mut rig.tw, o, &format!("+bay/stow #{}=#{}", a.0, o_hull.0));
    assert!(!refused.contains("is stowed"), "{refused}");
    let taken = run(&mut rig.tw, t, &format!("+bay/stow #{}=#{}", a.0, t_hull.0));
    assert!(
        taken.contains("is stowed in the Salt Moth's bay"),
        "{taken}"
    );
    assert_eq!(where_is(rig.tw.world(), a), Where::Bay { hull: t_hull });
    assert_eq!(held_for(&rig, a), None, "the hold clears");

    // 3. O sails home to Ceres with `d`: the port berths her in the workshop, held for T, with no
    //    carrier and no stand taken; O's slot frees.
    let ticket = file_and_launch(&mut rig, o, o_hull, "+ship/undock ceres leave=1");
    let _ = to_ic(&mut rig, ticket.plan.depart_ic + ticket.plan.total_s);
    let floor = station_room(&rig.tw, "ceres_port", "room.c14_dockmaster");
    let berth = match crate::driving_rig::store(&rig.tw).bay_hull(d) {
        Some(tacsim::port::bay_work::BayHullState::Present(row)) => row.berth.clone(),
        _ => None,
    }
    .expect("`d` is berthed at the docking");
    assert_eq!((berth.workshop, berth.carrier), (floor, None));
    assert_eq!(held_for(&rig, d), Some(PrincipalId::Object(t)));
    assert_ne!(where_is(rig.tw.world(), d), Where::Bay { hull: o_hull });

    // 4. T's carrier docks at Ceres and T stows `d` from the workshop floor.
    let ticket = file_and_launch(&mut rig, t, t_hull, "+ship/undock ceres");
    let _ = to_ic(&mut rig, ticket.plan.depart_ic + ticket.plan.total_s);
    let airlock = tacsim::port::ship::hull_view(rig.tw.world(), t_hull)
        .and_then(|view| view.airlock())
        .expect("the Salt Moth's airlock");
    teleport(&mut rig.tw, t, airlock);
    let left = run(&mut rig.tw, t, "+ship/leave");
    assert!(left.contains("You cycle the lock out"), "{left}");
    teleport(&mut rig.tw, t, floor);
    let shown = run(&mut rig.tw, t, &format!("+bay #{}", d.0));
    assert!(shown.contains("is held for Pilot"), "{shown}");
    let taken = run(&mut rig.tw, t, &format!("+bay/stow #{}=#{}", d.0, t_hull.0));
    assert!(
        taken.contains("is stowed in the Salt Moth's bay"),
        "{taken}"
    );
    assert_eq!(where_is(rig.tw.world(), d), Where::Bay { hull: t_hull });
    assert_eq!(held_for(&rig, d), None);
}

/// **RS.4's gate on Urvara Reach (512)**: story (b), extended.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rs4_story_b_the_vacated_hull_goes_home_on_urvara_reach() {
    story("urvara", "urvara_port", 512);
}

/// **RS.4's gate on Ahuna Flank (256)**: story (b), extended.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rs4_story_b_the_vacated_hull_goes_home_on_ahuna_flank() {
    story("ahuna", "ahuna_port", 256);
}

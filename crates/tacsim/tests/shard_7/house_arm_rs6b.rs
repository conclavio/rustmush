//! **Doc 41 cut RS.6b — Tycho Mutual flies a retrieval nobody accepted** (story (c)'s no-acceptor
//! variant; Q-41-11, Q-41-15), over the composed lane at 256 (Ahuna Flank) and 512 (Urvara Reach):
//!
//! Occator's spare rig lies dead on the site's ground; Occator posts its retrieval to Ceres; no
//! player accepts it in the window, and the clerk hands it to Tycho Mutual. Tycho's house arm then
//! flies the whole job on the port tick, as its dispatcher, through the production paths: the
//! lifter sails to the site, the crawler goes down the ramp, drives beside the rig, tows it to the
//! ramp and loads it, comes back up, the lifter sails home, and the rig is berthed in Ceres's
//! workshop — the delivery posted, the contract settled, the fee to Tycho.
//!
//! **What is a fixture.** The rig's power destroyed and its row on the site's index, by staff hand
//! (GC.5's fixture); the posting by the module's own `post` seam (Occator's, as the repossession
//! posts it). Every act of Tycho's is the production path.

use mush_core::ObjId;
use mush_org::PrincipalId;
use tacsim::surface::retrieval::{RetrievalStage, order_of};

use crate::driving_rig::{
    Rig, cid, fire, land_quietly, on_pad_as_at, port_map, ramp_at, staff_hand, stocked_tuned,
    to_cell_on,
};
use crate::field_host_rig::{second, tune};
use crate::site_record_rig::{staff, staff_member};

/// Short tow stages (`field_gc5`'s `quick`).
fn quick(cfg: &mut tacsim::TacsimConfig) {
    tune(cfg);
    cfg.field.idle_demote_s = 3_600;
    cfg.recovery.inspect_s = 30;
    cfg.recovery.attach_s = 30;
    cfg.recovery.load_s = 60;
}

/// The rig's power destroyed (`field_gc4`'s `powerless`): a casualty a tug can take.
fn powerless(rig: &mut Rig, unit: ObjId) {
    staff_hand(rig, |ctx| {
        let plan = mush_tactics::UnitMutationPlan::new(
            vec![mush_tactics::UnitMutationOp::SetComponentCondition {
                component: mush_tactics::unit::ComponentId::new("power").expect("an id"),
                condition: mush_tactics::ConditionId::new("destroyed").expect("a condition"),
            }],
            None,
            Some(smol_str::SmolStr::new_static("test:rs6b-powerless")),
        )
        .expect("a plan");
        mush_tactics::module::apply_plan(ctx, unit, &plan).expect("applied");
    });
}

/// Occator's principal and account.
fn occator(rig: &Rig) -> (PrincipalId, mush_economy::AccountId) {
    let map = port_map(&rig.tw, "ceres_port");
    let row = map
        .orgs
        .iter()
        .find(|(id, _)| id.as_str() == "occator")
        .map(|(_, row)| row.clone())
        .expect("Occator's row");
    (
        PrincipalId::Organization(mush_org::OrgId(row.org)),
        mush_economy::AccountId(row.account.expect("an account")),
    )
}

/// Move IC on to `target`, then fire a few times (`field_rs5`'s `skip_and_fire`).
fn skip_and_fire(rig: &mut Rig, target: i64) {
    let w = rig.tw.world();
    let rate = i64::from(w.world_timeline().current().rate.max(1));
    let ahead = target.saturating_sub(w.world_ic_now().at).max(0);
    let t = w.now().saturating_add((ahead + rate - 1) / rate);
    rig.tw.set_now(t);
    for _ in 0..3 {
        let (_, queued) = fire(rig);
        land_quietly(rig, &queued);
    }
}

/// **The job flown on `(venue, entry)`.**
#[allow(clippy::too_many_lines)]
fn story(venue: &str, entry: &str) {
    let mut rig = stocked_tuned(quick);
    // A pilot's carrier on the pad, so the site's ground is warm and its ramp is known.
    let (_, _, tvs) = on_pad_as_at(&mut rig, ("Pilot", 1, "Salt Moth"), (venue, entry), 1);
    let schema = mush_tactics::module::unit_of(rig.tw.world(), tvs[0])
        .expect("a rig")
        .schema()
        .clone();
    // Occator's spare rig, no lease on it.
    let casualty = rig
        .tw
        .world()
        .module_state::<mush_tactics::UnitStore>()
        .expect("the units")
        .units
        .iter()
        .filter(|(id, unit)| **id != tvs[0] && unit.schema() == &schema)
        .map(|(id, _)| *id)
        .find(|id| {
            crate::driving_rig::store(&rig.tw)
                .tv_lease_on(*id)
                .is_none()
        })
        .expect("a spare rig");
    let (rx, ry) = {
        let (x, y, _) = ramp_at(&rig.tw, entry);
        (x.div_euclid(1_000), y.div_euclid(1_000))
    };
    let at = (rx + 4, ry + 3);
    to_cell_on(&mut rig, entry, casualty, at);
    powerless(&mut rig, casualty);
    let s = staff_member(&mut rig);
    staff(
        &mut rig,
        s,
        &format!(
            "@tacsim/site {entry} leave #{} casualty {},{}",
            casualty.0, at.0, at.1
        ),
    );
    // The house is laid up (RS.6a) before the job.
    let ceres = cid("ceres_port");
    for _ in 0..60 {
        let (_, queued) = fire(&mut rig);
        land_quietly(&mut rig, &queued);
        if tacsim::port::house::dispatchers(rig.tw.world()).len() == 1 {
            break;
        }
    }
    let (lifter, _crawler) =
        tacsim::port::house::hulls(rig.tw.world(), &ceres).expect("Tycho's house");
    // Occator posts the job; nobody accepts it; at the window's end it is Tycho's.
    let poster = occator(&rig);
    let posted = staff_hand(&mut rig, |ctx| {
        tacsim::surface::retrieval::post(ctx, casualty, poster.clone(), ceres.clone(), None)
    });
    assert!(posted.is_ok(), "{posted:?}");
    let job = order_of(rig.tw.world(), casualty).expect("the job");
    skip_and_fire(&mut rig, job.accept_by_ic + 60);
    let job = order_of(rig.tw.world(), casualty).expect("the job");
    assert_eq!(job.stage, RetrievalStage::House, "{job:?}");
    // The house flies it, second by second (faster while the lifter is under way).
    let mut seen_at_site = false;
    for _ in 0..40_000 {
        if order_of(rig.tw.world(), casualty).is_some_and(|o| o.delivered.is_some()) {
            break;
        }
        if matches!(
            tacsim::port::ship::berth_of(rig.tw.world(), lifter),
            tacsim::port::ship::Berth::Docked { map, .. } if map.entry.as_str() == entry
        ) {
            seen_at_site = true;
        }
        let under_way = matches!(
            tacsim::port::ship::berth_of(rig.tw.world(), lifter),
            tacsim::port::ship::Berth::UnderWay
        );
        if under_way {
            let t = rig.tw.world().now().saturating_add(30);
            rig.tw.set_now(t);
        }
        let _ = second(&mut rig);
    }
    let done = order_of(rig.tw.world(), casualty).expect("the job");
    assert!(seen_at_site, "the lifter landed at {entry}");
    let delivered = done.delivered.clone().unwrap_or_else(|| {
        panic!(
            "delivered by Tycho: {done:?}; the casualty {:?}; the lifter {:?}",
            tacsim::surface::act::where_is(rig.tw.world(), casualty),
            tacsim::port::ship::berth_of(rig.tw.world(), lifter)
        )
    });
    assert_eq!(delivered.port, ceres);
    // Berthed in Ceres's workshop.
    let floor = crate::driving_rig::station_room(&rig.tw, "ceres_port", "room.c14_dockmaster");
    assert_eq!(
        rig.tw.world().get(casualty).and_then(|o| o.location),
        Some(floor),
        "the rig is berthed in the workshop"
    );
}

/// **RS.6b's gate on Ahuna Flank (256).**
#[test]
fn rs6b_tycho_flies_an_unaccepted_retrieval_on_ahuna_flank() {
    story("ahuna", "ahuna_port");
}

/// **RS.6b's gate on Urvara Reach (512).**
#[test]
fn rs6b_tycho_flies_an_unaccepted_retrieval_on_urvara_reach() {
    story("urvara", "urvara_port");
}

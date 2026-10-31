//! **Doc 32 cut CF.3 — the carrier as a unit**, over CF.2's composed world (`driving`'s keyed
//! rig on Ahuna Flank) and its hand-pumped field (`field_host`'s helpers).
//!
//! The cut's gate: a Dray-P settled on Ahuna Flank's pad is admitted to a field at its pad pose
//! with its real sections; its pilot, aboard, reads as aboard the carrier; a lift from the pad is
//! refused while the field holds it, without the leave-behind confirmation (Q-H10, F-6); an armed
//! hull's fire hits it; and after the field demotes the damage is on the hull's own unit record
//! and `+ship` reads it in port. F-6's own story is the second test.

use mush_core::ObjId;
use tacsim::field::CrewLocation;

use crate::driving::{
    cid, command, land, launched, on_pad, prospector, run, staff_hand, stocked_tuned, to_cell, warm,
};
use crate::field_host::{
    armed, egress, frame, seat, second, site_record, task, tune, until, weapon_of,
};

/// **Strip `unit`'s armour to its floor**, leaving its internals whole, so the next rounds that
/// land are section damage (a fixture, so the gate fights in seconds).
fn unarmoured(rig: &mut crate::driving::Rig, unit: ObjId) {
    staff_hand(rig, |ctx| {
        let cells = mush_tactics::module::unit_of(ctx.w, unit)
            .expect("the unit")
            .clone();
        let ops: Vec<mush_tactics::UnitMutationOp> = cells
            .cells()
            .filter_map(|(addr, cell)| match addr {
                mush_tactics::CellAddr::Facet { cell: id, .. }
                    if id.as_str() == tacsim::definitions::unit::ARMOR_CELL =>
                {
                    Some(mush_tactics::UnitMutationOp::SetCell {
                        addr: addr.clone(),
                        value: cell.min,
                    })
                }
                _ => None,
            })
            .collect();
        let plan = mush_tactics::UnitMutationPlan::new(
            ops,
            None,
            Some(smol_str::SmolStr::new_static("test:cf3-unarmoured")),
        )
        .expect("a plan");
        mush_tactics::module::apply_plan(ctx, unit, &plan).expect("applied");
    });
}

/// Whether any section of `cells` carries less structure than it was built with.
fn hurt(cells: &mush_tactics::TacticalUnit) -> bool {
    cells.cells().any(|(addr, cell)| {
        matches!(
            addr,
            mush_tactics::CellAddr::Section { cell: id, .. }
                if id.as_str() == tacsim::definitions::unit::INTERNALS_CELL
        ) && cell.current < cell.max
    })
}

/// **CF.3's gate** (doc 32 §6; Q-CF-19, Q-CF-38, Q-H10).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn cf3_a_carrier_takes_a_hit_in_a_field_and_the_damage_shows_in_port() {
    let mut rig = stocked_tuned(tune);
    // A's Dray-P on Ahuna Flank's pad, A aboard, A's prospector out on the ground to the south of
    // B's gun; B flies an armed hull from a remote cab in port, west of the pad.
    let (a, a_row, a_tvs) = on_pad(&mut rig, 1);
    let carrier = ObjId(a_row.hull);
    launched(&mut rig, a, a_tvs[0]);
    to_cell(&mut rig, a_tvs[0], (120, 120));
    let (b, _) = prospector(&mut rig, "Other", 2, "Tally Moth");
    let hull_b = armed(&mut rig, "Grey Hound", (120, 128), 90_000);
    seat(&mut rig, hull_b, b);
    unarmoured(&mut rig, carrier);
    let before = mush_tactics::module::unit_of(rig.tw.world(), carrier)
        .expect("the carrier's unit record")
        .clone();
    let placed = mush_sim::pose_of(rig.tw.world(), carrier).expect("the carrier is placed");
    warm(&mut rig);
    let mut rx = egress(&mut rig);
    let mut lines = Vec::new();
    for _ in 0..3 {
        let _ = second(&mut rig);
    }

    // (1) B's fire order at A's prospector promotes the site; everyone on the ground is admitted,
    // the carrier on its pad among them.
    let (out, queued) = command(&mut rig, b, "+field/fire S");
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    land(&mut rig, &queued[0]).expect("the promotion lands");
    assert!(
        until(&mut rig, &mut rx, &mut lines, 10, |rig, _| task(rig)
            .is_some())
        .await,
        "the field's task opens"
    );
    let mark = mush_sim::deployed::mark_of(rig.tw.world(), carrier)
        .expect("the carrier is in the field (Q-CF-19)");
    assert_eq!(mark.field.as_ref().map(|field| field.ordinal), Some(1));
    assert_eq!(
        tacsim::field::crew_location(rig.tw.world(), a),
        Some(CrewLocation::Carrier { hull: carrier }),
        "A, aboard, is aboard the carrier (Q-CF-38)"
    );

    // (2) A lift from the pad while the field holds the hull is refused without the leave-behind
    // confirmation, naming the rig it would leave (doc 32 F-6).
    let lift = run(&mut rig.tw, a, "+ship/lift ceres");
    assert!(lift.contains("add leave=1"), "{lift}");

    // (3) B locks the carrier on its pad and fires until rounds have left the gun at it.
    let gun = weapon_of(&rig, hull_b);
    let rounds = |rig: &crate::driving::Rig| -> Option<u32> {
        frame(rig, b).and_then(|frame| {
            frame
                .weapons
                .iter()
                .find(|weapon| weapon.id.as_str() == gun)
                .map(|weapon| weapon.rounds)
        })
    };
    let pad_contact = |rig: &crate::driving::Rig| {
        frame(rig, b).and_then(|frame| {
            frame
                .contacts
                .iter()
                .find(|contact| {
                    contact
                        .cell
                        .is_some_and(|(x, y)| x.abs_diff(128) <= 1 && y.abs_diff(128) <= 1)
                })
                .cloned()
        })
    };
    assert!(
        until(&mut rig, &mut rx, &mut lines, 20, |rig, _| pad_contact(rig)
            .is_some()
            && rounds(rig).is_some())
        .await,
        "B's sensors hold the carrier on the pad"
    );
    let full = rounds(&rig).expect("B's gun");
    let fired = until(&mut rig, &mut rx, &mut lines, 90, |rig, _| {
        if let Some(contact) = pad_contact(rig)
            && frame(rig, b).is_some_and(|frame| frame.lock != Some(contact.id))
        {
            let _ = run(&mut rig.tw, b, &format!("LOCK {}", contact.designator));
        }
        let _ = run(&mut rig.tw, b, &format!("FIRE {gun}"));
        let locked = pad_contact(rig)
            .is_some_and(|contact| frame(rig, b).is_some_and(|f| f.lock == Some(contact.id)));
        locked && rounds(rig).is_some_and(|left| full.saturating_sub(left) >= 4)
    })
    .await;
    assert!(fired, "B fires at the carrier");

    // (4) Nobody fires any more: the field disengages and demotes.
    assert!(
        until(&mut rig, &mut rx, &mut lines, 60, |rig, _| {
            task(rig).is_none()
                && site_record(rig).is_some_and(|record| record.open_field().is_none())
        })
        .await,
        "the field demotes after its quiet interval"
    );
    assert!(
        !mush_sim::deployed::is_deployed(rig.tw.world(), carrier),
        "the carrier is checked in"
    );
    assert_eq!(
        tacsim::field::crew_location(rig.tw.world(), a),
        Some(CrewLocation::Carrier { hull: carrier }),
        "A's crew record keeps them aboard the carrier after the field (doc 32 cut CF.5)"
    );
    assert_eq!(
        mush_sim::pose_of(rig.tw.world(), carrier),
        Some(placed),
        "the carrier went home where it was settled"
    );

    // (5) The damage persisted through the IC check-in into the hull's own record, and +ship
    // reads it in port.
    let after = mush_tactics::module::unit_of(rig.tw.world(), carrier)
        .expect("the carrier's unit record")
        .clone();
    assert!(
        after.revision() > before.revision(),
        "the field's damage is a new revision of the carrier"
    );
    assert!(hurt(&after), "a section of the carrier came home damaged");
    let ship = run(&mut rig.tw, a, "+ship");
    assert!(ship.contains("Damage: "), "{ship}");
    // The surface's pad is the surface's again: the lift's field gate is gone.
    let lift = run(&mut rig.tw, a, "+ship/lift ceres");
    assert!(!lift.contains("is in the field"), "{lift}");
}

/// **A carrier is never towed** (FO-13): every shipped carrier is past twice the tonnage of
/// every shipped mobile design, and the named gate refuses it first.
#[test]
fn every_carrier_is_past_twice_any_tugs_tonnage() {
    let mut rig = stocked_tuned(|_| {});
    let carriers = [
        "tacsim.unit.dray",
        "tacsim.unit.dray_p",
        "tacsim.unit.halcyon",
    ];
    let store = rig
        .tw
        .world()
        .module_state::<tacsim::TacsimStore>()
        .expect("the layer");
    let specs = &store.definitions().specs;
    let mut checked = 0;
    for carrier in carriers {
        let Some(spec) = specs.get(&cid(carrier)) else {
            continue;
        };
        let heavy = tacsim::field_ops::dry_mass_kg_for_spec(spec);
        for (id, tug) in specs {
            if carriers.contains(&id.as_str())
                || tacsim::field_ops::base_speed_cms_for_spec(tug) == 0
            {
                continue;
            }
            let light = tacsim::field_ops::dry_mass_kg_for_spec(tug);
            assert!(
                tacsim::field_ops::tow_mass_ratio_permille(light, heavy).is_err(),
                "{id} ({light} kg) could tow {carrier} ({heavy} kg)"
            );
            checked += 1;
        }
    }
    assert!(checked > 0, "the lane binds a carrier and a tug");
    let (_, row) = prospector(&mut rig, "Pilot", 1, "Salt Moth");
    let refused = tacsim::field::carrier::refuse_tow(rig.tw.world(), ObjId(row.hull))
        .expect("a carrier is never towed");
    assert!(refused.contains("never towed"), "{refused}");
}

/// **Doc 32 F-6, the carrier's departure from a running field** (ruled 2026-10-31; built in cut
/// CF.7): without the leave-behind confirmation a live lift is refused, naming the rig it would
/// leave; with it the carrier is removed from the field outright, its pilot's departure files,
/// and the rig it left stays in the field.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn f6_a_carrier_lifts_out_of_a_running_field_only_on_the_confirmation() {
    let mut rig = stocked_tuned(tune);
    // CF.3's ground: A aboard the Dray-P on the pad, A's prospector out to the south of B's gun.
    let (a, a_row, a_tvs) = on_pad(&mut rig, 1);
    let carrier = ObjId(a_row.hull);
    let rig_a = a_tvs[0];
    launched(&mut rig, a, rig_a);
    to_cell(&mut rig, rig_a, (120, 120));
    let (b, _) = prospector(&mut rig, "Other", 2, "Tally Moth");
    let hull_b = armed(&mut rig, "Grey Hound", (120, 128), 90_000);
    seat(&mut rig, hull_b, b);
    warm(&mut rig);
    let mut rx = egress(&mut rig);
    let mut lines = Vec::new();
    for _ in 0..3 {
        let _ = second(&mut rig);
    }
    let (out, queued) = command(&mut rig, b, "+field/fire S");
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    land(&mut rig, &queued[0]).expect("the promotion lands");
    assert!(
        until(&mut rig, &mut rx, &mut lines, 10, |rig, _| task(rig)
            .is_some()
            && tacsim::field::field_of_unit(rig.tw.world(), carrier)
                .is_some())
        .await,
        "the field opens with the carrier in it"
    );

    // Without the confirmation: refused, naming the rig and the remedy; nothing is filed.
    let (refused, queued) = command(&mut rig, a, "+ship/lift ceres");
    assert!(refused.contains("add leave=1"), "{refused}");
    assert!(queued.is_empty(), "a refused lift submits nothing");
    assert!(tacsim::field::field_of_unit(rig.tw.world(), carrier).is_some());

    // With it: the carrier breaks off at the next pulse, and the departure files as A after.
    let (asked, queued) = command(&mut rig, a, "+ship/lift ceres leave=1");
    assert!(asked.contains("breaks off"), "{asked}");
    assert!(
        queued.is_empty(),
        "nothing is filed while the field holds the hull"
    );
    let filed = |rig: &crate::driving::Rig| match rig
        .tw
        .world()
        .module_state::<tacsim::TacsimStore>()
        .map(|store| store.voyage_state(carrier))
    {
        Some(tacsim::port::voyage::VoyageState::Present(voyage)) => voyage
            .pending
            .as_ref()
            .map(|pending| pending.left_behind.clone()),
        _ => None,
    };
    assert!(
        until(&mut rig, &mut rx, &mut lines, 20, |rig, _| filed(rig)
            .is_some())
        .await,
        "the departure files once the carrier is out of the field"
    );
    assert_eq!(
        filed(&rig),
        Some(vec![rig_a]),
        "the filed voyage records the rig left"
    );
    assert!(
        !mush_sim::deployed::is_deployed(rig.tw.world(), carrier),
        "the carrier is checked in, out of the field"
    );
    assert!(
        site_record(&mut rig).is_some_and(|record| record.exited(carrier)),
        "the open promotion names the carrier's exit"
    );
    // The field runs on, and the rig it left stays in it.
    assert!(task(&rig).is_some(), "the field carries on");
    let live = tacsim::field::live_on(rig.tw.world(), &cid(crate::driving::AHUNA))
        .expect("the live record");
    assert!(live.units.iter().all(|row| row.unit != carrier));
    assert!(live.units.iter().any(|row| row.unit == rig_a));
    assert!(tacsim::field::field_of_unit(rig.tw.world(), rig_a).is_some());
}

//! **Doc 36 cut GC.3 — remains, and the carrier's loss** (Q-36-8, Q-36-18, Q-36-47), over the
//! composed world (`driving`'s keyed rig on Ahuna Flank), the arena's egress pumped by hand as
//! `field_host.rs` pumps it.
//!
//! - **Close-destroyed** (Q-36-18): a prospector out on the ground is an irrecoverable hull when
//!   a field over it demotes. It leaves the site's unit index and the surface, a remains object
//!   with its mass lies where it fell, its epoch closes `Destroyed` on its rig record and its
//!   black box, and the party that watched it learns the remains.
//! - **The carrier's loss table** (Q-36-8): a Dray-P whose drive is destroyed refuses lift; one
//!   whose hull falls in a field is a wreck that never lifts, strands the crew aboard it and
//!   wrecks the rig in its bay.
//!
//! **What is a fixture.** A field reaches `Irrecoverable` only when a hull goes down with every
//! section destroyed (the IC check-in reads a section at its internals' floor as destroyed), as
//! the CF.7 story's walker does. The first gate does not fight every section down: it marks one
//! of the prospector's sections `irrecoverable` by staff hand before the field, and everything
//! after the mark runs the production field. The drive gate stands a destroyed drive in by staff
//! hand too, as a field's damage leaves it (the section's `destroyed` condition, its internals at
//! their floor). The hull's loss is fought: B's gun takes the carrier's last point of hull
//! structure in the field.

use mush_core::ObjId;
use tacsim::arena::ledger::{EventKind, LedgerEvent};
use tacsim::carrier_damage::{CarrierLoss, CarrierPart};
use tacsim::field::CrewLocation;
use tacsim::surface::black_box::BlackBoxLine;
use tacsim::surface::site_record::{GroundKind, KnownWhat};

use crate::driving_rig::{
    command, land, launched, launched_composed, on_pad, on_pad_as, prospector, record_of, run,
    staff_hand, stocked_tuned, to_cell, warm,
};
use crate::field_host_rig::{
    armed, egress, frame, seat, second, second_composed, site_record, task, tune, until,
    until_composed, weapon_of,
};

/// **Write `condition` on `section` of `unit`, and its internals to `internals`** (or leave them
/// when `None`): the staff fixture standing in for damage.
fn set_section(
    rig: &mut crate::driving_rig::Rig,
    unit: ObjId,
    section: &str,
    condition: Option<&str>,
    internals: Option<i64>,
) {
    staff_hand(rig, |ctx| {
        let cells = mush_tactics::module::unit_of(ctx.w, unit)
            .expect("the unit")
            .clone();
        let id = mush_tactics::SectionId::new(section).expect("a section id");
        assert!(cells.section(&id).is_some(), "#{} has a {section}", unit.0);
        let mut ops = Vec::new();
        if let Some(condition) = condition {
            ops.push(mush_tactics::UnitMutationOp::SetSectionCondition {
                section: id.clone(),
                condition: mush_tactics::ConditionId::new(condition).expect("a condition"),
            });
        }
        if let Some(value) = internals {
            ops.push(mush_tactics::UnitMutationOp::SetCell {
                addr: mush_tactics::CellAddr::Section {
                    section: id,
                    cell: mush_tactics::CellId::new(tacsim::definitions::unit::INTERNALS_CELL)
                        .expect("a cell id"),
                },
                value,
            });
        }
        let plan = mush_tactics::UnitMutationPlan::new(
            ops,
            None,
            Some(smol_str::SmolStr::new_static("test:gc3-damage")),
        )
        .expect("a plan");
        mush_tactics::module::apply_plan(ctx, unit, &plan).expect("applied");
    });
}

/// **Strip `unit`'s armour to its floor** (CF.3's fixture), so the next rounds that land are
/// section damage.
fn unarmoured(rig: &mut crate::driving_rig::Rig, unit: ObjId) {
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
            Some(smol_str::SmolStr::new_static("test:gc3-unarmoured")),
        )
        .expect("a plan");
        mush_tactics::module::apply_plan(ctx, unit, &plan).expect("applied");
    });
}

/// Whether the field has demoted: no task, and the record's promotion closed.
fn demoted(rig: &mut crate::driving_rig::Rig) -> bool {
    task(rig).is_none() && site_record(rig).is_some_and(|record| record.open_field().is_none())
}

/// `hull`'s black box, its open tail.
pub(crate) fn black_box(rig: &crate::driving_rig::Rig, hull: ObjId) -> Vec<BlackBoxLine> {
    rig.tw
        .world()
        .module_state::<tacsim::TacsimStore>()
        .expect("the layer")
        .black_box_lines(hull)
        .0
        .into_iter()
        .map(|(_, line)| line)
        .collect()
}

/// Whether `unit` is a wreck on its own record.
fn wrecked(rig: &crate::driving_rig::Rig, unit: ObjId) -> bool {
    mush_tactics::module::unit_of(rig.tw.world(), unit).is_some_and(tacsim::arena::damage::is_wreck)
}

/// **GC.3's first gate** (Q-36-18, Q-CF-25 c): an irrecoverable hull leaves remains and closes
/// its epoch destroyed. The launch and every tick's landing go through the composed path (doc
/// 36a lane 4: `launched_composed`, `until_composed`), production's, so the black box holds the
/// epoch's open line and the close line is asserted, not skipped.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn gc3_an_irrecoverable_hull_leaves_remains_and_closes_destroyed() {
    let mut rig = stocked_tuned(|cfg| {
        tune(cfg);
        cfg.field.disengage_s = 15;
        cfg.field.idle_demote_s = 3_600;
    });
    // GC.2's ground: A in an armed hull, C's prospector out four cells east of it.
    let (a, _, _) = on_pad(&mut rig, 1);
    let (c, _, c_tvs) = on_pad_as(&mut rig, "Third", 3, "Lone Moth", 1);
    let lost = c_tvs[0];
    launched_composed(&mut rig, c, lost);
    to_cell(&mut rig, lost, (114, 124));
    let hull = armed(&mut rig, "Red Hound", (110, 124), 90_000);
    seat(&mut rig, hull, a);
    let (epoch, unrecorded) = record_of(&mut rig.tw, lost)
        .and_then(|record| record.sortie)
        .map(|open| (open.id, open.unrecorded))
        .expect("the prospector's epoch is open on its rig record");
    assert!(
        !unrecorded,
        "the composed launch writes the epoch's open line"
    );
    assert!(
        black_box(&rig, lost)
            .iter()
            .any(|line| matches!(line, BlackBoxLine::MindSealed { .. })),
        "the open line is on the black box"
    );
    // The catastrophe the kernel does not model yet (this file's head).
    set_section(&mut rig, lost, "boom", Some("irrecoverable"), None);
    let mass_kg = tacsim::carrier_damage::mass_kg(rig.tw.world(), lost).expect("a mass");
    assert!(mass_kg > 0);
    warm(&mut rig);
    let mut rx = egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();
    for _ in 0..3 {
        let _ = second_composed(&mut rig);
    }

    // A marks the prospector Hostile; the site promotes with it on the ground, and nobody fires.
    let (out, queued) = command(&mut rig, a, "+field/mark E=hostile");
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    land(&mut rig, &queued[0]).expect("the promotion lands");
    assert!(
        until_composed(&mut rig, &mut rx, &mut lines, 15, |rig, _| task(rig)
            .is_some())
        .await,
        "the field's task opens"
    );
    assert!(
        tacsim::field::field_of_unit(rig.tw.world(), lost).is_some(),
        "the prospector is in the field"
    );
    assert!(
        until_composed(&mut rig, &mut rx, &mut lines, 60, |rig, _| demoted(rig)).await,
        "the field demotes"
    );

    // The record: no index entry, one remains object of the hull with its mass, where it stood.
    let record = site_record(&mut rig).expect("the site record");
    assert!(
        record.units.iter().all(|left| left.unit != lost),
        "an irrecoverable hull is no casualty or wreck on the index: {:?}",
        record.units
    );
    let remains: Vec<_> = record
        .objects
        .iter()
        .filter(
            |object| matches!(object.kind, GroundKind::Remains { of: Some(of), .. } if of == lost),
        )
        .collect();
    assert_eq!(remains.len(), 1, "one remains: {:?}", record.objects);
    assert!(
        matches!(
            remains[0].kind,
            GroundKind::Remains { mass_kg: kg, merged: 1, .. } if kg == mass_kg
        ),
        "{:?}",
        remains[0]
    );
    assert_eq!(remains[0].pose.cell(), (114, 124), "where it fell");
    assert_eq!(remains[0].footprint, 1);
    // The surface no longer holds the hull.
    assert!(
        mush_sim::pose_of(rig.tw.world(), lost).is_none(),
        "the hull is off the ground"
    );
    // The epoch closed destroyed on the rig record and on the black box.
    let rig_record = record_of(&mut rig.tw, lost).expect("the rig record");
    assert!(rig_record.sortie.is_none(), "the epoch is closed");
    assert!(rig_record.excursion.is_none(), "nothing of its work stands");
    // The close line rides the close act through the black box's participant, on the composed
    // path the host's owed close was landed on.
    let closed = black_box(&rig, lost)
        .into_iter()
        .rev()
        .find_map(|line| match line {
            BlackBoxLine::SortieClosed { sortie, close, .. } => Some((sortie, close)),
            BlackBoxLine::MindSealed { .. } => None,
        });
    assert_eq!(
        closed,
        Some((
            epoch,
            mush_roll::commerce::ic_claims::IcCloseKind::Destroyed
        )),
        "the black box closes the epoch destroyed"
    );
    // A watched it fall from four cells: A's party knows the remains, not a wreck.
    assert!(
        record
            .known_by(a)
            .iter()
            .any(|item| item.0 == KnownWhat::Remains && item.cell() == (114, 124)),
        "{:?}",
        record.known_by(a)
    );
    // Its armed watcher is untouched: a mobile hull leaves nothing on the index.
    assert!(record.units.iter().all(|left| left.unit != hull));
    let _ = c;
}

/// **GC.3's second gate** (Q-36-8): a carrier with its drive destroyed cannot lift, and the
/// table reads the drive and only the drive.
#[test]
fn gc3_a_carrier_with_its_drive_destroyed_cannot_lift() {
    let mut rig = stocked_tuned(tune);
    let (a, row, _) = on_pad(&mut rig, 1);
    let carrier = ObjId(row.hull);
    let whole = tacsim::carrier_damage::damage_of(rig.tw.world(), carrier).expect("a reading");
    assert!(whole.losses().is_empty(), "a whole carrier loses nothing");
    // The drive as a field leaves it: destroyed, its internals at their floor.
    set_section(&mut rig, carrier, "drive", Some("destroyed"), Some(0));
    let damage = tacsim::carrier_damage::damage_of(rig.tw.world(), carrier).expect("a reading");
    assert!(damage.is_down(CarrierPart::Drive));
    assert!(!damage.is_down(CarrierPart::Hull));
    assert!(damage.has(CarrierLoss::RefusesLift));
    assert!(!damage.has(CarrierLoss::Wrecked));
    // The live lift is refused, naming the drive, and nothing is filed.
    let (out, queued) = command(&mut rig, a, "+ship/lift ceres");
    assert!(out.contains("drive is destroyed"), "{out}");
    assert!(queued.is_empty(), "a refused lift submits nothing: {out}");
    let pending = rig
        .tw
        .world()
        .module_state::<tacsim::TacsimStore>()
        .map(|store| store.voyage_state(carrier));
    assert!(
        !matches!(
            pending,
            Some(tacsim::port::voyage::VoyageState::Present(voyage)) if voyage.pending.is_some()
        ),
        "no passage is pending"
    );
    // The carrier stays on its pad.
    let lifted = run(&mut rig.tw, a, "+ship");
    assert!(!lifted.contains("under way"), "{lifted}");
}

/// **The hull's loss, fought** (Q-36-8): B's gun takes the last point of the Dray-P's hull in a
/// field. At demotion the carrier is a wreck on the index, its pilot aboard is stranded, the rig
/// in its bay is a wreck, and a lift meets the wreck's refusal.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn gc3_a_carrier_whose_hull_falls_strands_its_crew_and_wrecks_its_bay() {
    let mut rig = stocked_tuned(tune);
    // CF.3's ground, with a second rig left in the bay.
    let (a, a_row, a_tvs) = on_pad(&mut rig, 2);
    let carrier = ObjId(a_row.hull);
    launched(&mut rig, a, a_tvs[0]);
    to_cell(&mut rig, a_tvs[0], (120, 120));
    let in_bay = a_tvs[1];
    let (b, _) = prospector(&mut rig, "Other", 2, "Tally Moth");
    let hull_b = armed(&mut rig, "Grey Hound", (120, 128), 90_000);
    seat(&mut rig, hull_b, b);
    unarmoured(&mut rig, carrier);
    set_section(&mut rig, carrier, "hull", None, Some(1));
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
            .is_some())
        .await,
        "the field's task opens"
    );

    // B locks the carrier on its pad and fires until its hull falls.
    let gun = weapon_of(&rig, hull_b);
    let pad_contact = |rig: &crate::driving_rig::Rig| {
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
    let destroyed = |lines: &[LedgerEvent]| {
        lines.iter().any(
            |line| matches!(line.kind, EventKind::UnitDestroyed { unit, .. } if unit == carrier),
        )
    };
    let fell = until(&mut rig, &mut rx, &mut lines, 120, |rig, lines| {
        if destroyed(lines) {
            return true;
        }
        if let Some(contact) = pad_contact(rig)
            && frame(rig, b).is_some_and(|frame| frame.lock != Some(contact.id))
        {
            let _ = run(&mut rig.tw, b, &format!("LOCK {}", contact.designator));
        }
        let _ = run(&mut rig.tw, b, &format!("FIRE {gun}"));
        false
    })
    .await;
    assert!(fell, "B's fire takes the carrier's hull");
    assert!(
        until(&mut rig, &mut rx, &mut lines, 60, |rig, _| demoted(rig)).await,
        "the field demotes after its quiet interval"
    );

    // The table, read off the checked-in record.
    assert!(wrecked(&rig, carrier), "the carrier is a wreck");
    let damage = tacsim::carrier_damage::damage_of(rig.tw.world(), carrier).expect("a reading");
    for loss in [
        CarrierLoss::RefusesLift,
        CarrierLoss::Wrecked,
        CarrierLoss::CrewStranded,
        CarrierLoss::BayRigsWrecked,
    ] {
        assert!(damage.has(loss), "{loss:?}");
    }
    let record = site_record(&mut rig).expect("the site record");
    assert!(
        record.units.iter().any(|left| left.unit == carrier
            && left.condition == tacsim::surface::site_record::LeftCondition::Wreck),
        "the carrier is a wreck on the index: {:?}",
        record.units
    );
    // Its pilot aboard is stranded, and may ask for extraction.
    assert_eq!(
        tacsim::field::crew_location(rig.tw.world(), a),
        Some(CrewLocation::Stranded {
            site: crate::driving_rig::cid(crate::driving_rig::AHUNA),
            unit: carrier,
        }),
        "A, aboard the wreck, is stranded"
    );
    // The rig in its bay is a wreck where the carrier stands; the rig that was out is not.
    assert!(wrecked(&rig, in_bay), "the bay rig is a wreck");
    assert!(!wrecked(&rig, a_tvs[0]), "the rig out on the ground is not");
    // The wreck never lifts.
    let (out, queued) = command(&mut rig, a, "+ship/lift ceres");
    assert!(out.contains("is a wreck"), "{out}");
    assert!(queued.is_empty(), "{out}");
}

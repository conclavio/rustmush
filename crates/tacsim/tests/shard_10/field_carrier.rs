//! **Doc 32 cut CF.3 — the carrier as a unit**, over CF.2's composed world (`driving`'s keyed
//! rig on Ahuna Flank) and its hand-pumped field (`field_host`'s helpers).
//!
//! The cut's gate: a Dray-P settled on Ahuna Flank's pad is admitted to a field at its pad pose
//! with its real sections; its pilot, aboard, reads as aboard the carrier; a lift from the pad is
//! refused while the field holds it, without the leave-behind confirmation (Q-H10, F-6); an armed
//! hull's fire hits it; and after the field demotes the damage is on the hull's own unit record
//! and `+ship` reads it in port. F-6's own story is the second test.

use mush_core::{GOD, ObjId};
use tacsim::TacsimConfig;
use tacsim::arena::ledger::LedgerEvent;
use tacsim::arena::party::PartyId;
use tacsim::arena_task::ArenaOut;
use tacsim::field::{CrewLocation, FieldLive};
use tacsim::port::Ticket;
use tokio::sync::mpsc::Receiver;

use crate::driving_rig::crash;
use crate::driving_rig::{
    Rig, cid, command, en_route, land, launched, on_pad, prospector, run, staff_hand,
    stocked_tuned, to_cell, to_ic, warm,
};
use crate::field_host_rig::{
    armed, egress, frame, seat, second, site_record, task, tune, until, weapon_of,
};

/// **Strip `unit`'s armour to its floor**, leaving its internals whole, so the next rounds that
/// land are section damage (a fixture, so the gate fights in seconds).
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
    let rounds = |rig: &crate::driving_rig::Rig| -> Option<u32> {
        frame(rig, b).and_then(|frame| {
            frame
                .weapons
                .iter()
                .find(|weapon| weapon.id.as_str() == gun)
                .map(|weapon| weapon.rounds)
        })
    };
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

/// **A carrier is not towed yet** (FO-13; doc 36 cut GC.3, Q-36-8): every shipped carrier is
/// past twice the tonnage of every shipped mobile design, and the named gate wants a tug of at
/// least half the carrier's.
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
    // Doc 36 cut GC.3 (Q-36-8 as ruled): a carrier wants a tug of at least half its tonnage, and
    // the gate names the tonnage; a rig is far short of it.
    let (_, row, tvs) = on_pad(&mut rig, 1);
    let carrier = ObjId(row.hull);
    let refused = tacsim::field::carrier::refuse_tow(rig.tw.world(), carrier, tvs[0])
        .expect("a rig cannot tow a carrier");
    assert!(
        refused.contains("carrier of 240 t") && refused.contains("at least 120 t"),
        "{refused}"
    );
    assert_eq!(
        tacsim::field::carrier::refuse_tow(rig.tw.world(), tvs[0], carrier),
        None,
        "the gate reads only a carrier"
    );
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
    let filed = |rig: &crate::driving_rig::Rig| match rig
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
    let live = tacsim::field::live_on(rig.tw.world(), &cid(crate::driving_rig::AHUNA))
        .expect("the live record");
    assert!(live.units.iter().all(|row| row.unit != carrier));
    assert!(live.units.iter().any(|row| row.unit == rig_a));
    assert!(tacsim::field::field_of_unit(rig.tw.world(), rig_a).is_some());
}

/// **Doc 36 cut GC.1b's gate: F-6 arrival** (Q-36-4, Q-CF-16, Q-CF-17, Q-CF-19). A second
/// Dray-P is under way to Ahuna Flank when CF.3's field opens there, and settles on the pad
/// mid-field. The host owes it its admission (GC.1) and takes it in (GC.1b): it is checked out
/// under the field's mark, it has a row on the live record in a party of its own, the kernel
/// joins it at the next pulse (its `mind_sealed` line on the black box) beside the pad, where the
/// armed hull's sensors hold it as a second contact; its arrival is a row in the presence
/// picture, the field runs on, and its rig's launch is refused while the field runs.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn gc1_a_carrier_arriving_mid_field_is_admitted_at_its_pad_and_cannot_launch() {
    let mut rig = stocked_tuned(tune);
    let (a, _, a_tvs) = on_pad(&mut rig, 1);
    launched(&mut rig, a, a_tvs[0]);
    to_cell(&mut rig, a_tvs[0], (120, 120));
    let (b, _) = prospector(&mut rig, "Other", 2, "Tally Moth");
    let hull_b = armed(&mut rig, "Grey Hound", (120, 128), 90_000);
    seat(&mut rig, hull_b, b);
    let (late, late_row, _, ticket) = en_route(&mut rig, "Late", 3, "Late Moth");
    let late_hull = ObjId(late_row.hull);
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
        "the field opens"
    );
    let site = cid(crate::driving_rig::AHUNA);
    assert!(tacsim::field::carrier::owed_arrivals(rig.tw.world(), &site).is_empty());
    let opening = tacsim::field::live_on(rig.tw.world(), &site).expect("the live record");
    assert!(opening.units.iter().all(|row| row.unit != late_hull));
    // Every contact B's sensors hold within a cell of the pad (CF.3's carrier stands on it).
    let near_pad = |rig: &crate::driving_rig::Rig| -> usize {
        frame(rig, b).map_or(0, |frame| {
            frame
                .contacts
                .iter()
                .filter(|contact| {
                    contact
                        .cell
                        .is_some_and(|(x, y)| x.abs_diff(128) <= 1 && y.abs_diff(128) <= 1)
                })
                .count()
        })
    };
    assert!(
        until(&mut rig, &mut rx, &mut lines, 20, |rig, _| near_pad(rig)
            == 1)
        .await,
        "B's sensors hold the one carrier on the pad"
    );

    // The late hull settles on the pad while the field runs.
    let _ = to_ic(&mut rig, ticket.plan.depart_ic + ticket.plan.total_s);
    assert!(
        matches!(
            tacsim::port::ship::berth_of(rig.tw.world(), late_hull),
            tacsim::port::ship::Berth::Docked { map, .. } if map.entry.as_str() == crate::driving_rig::AHUNA
        ),
        "the late hull is settled on the pad"
    );
    // (1) The host takes it in: checked out under the field's mark, a row of its own.
    assert!(
        until(&mut rig, &mut rx, &mut lines, 10, |rig, _| {
            tacsim::field::field_of_unit(rig.tw.world(), late_hull) == Some(site.clone())
        })
        .await,
        "the arrival is checked out into the field"
    );
    assert!(
        tacsim::field::carrier::owed_arrivals(rig.tw.world(), &site).is_empty(),
        "nothing is owed once it is taken in"
    );
    let live = tacsim::field::live_on(rig.tw.world(), &site).expect("the live record");
    let row = live
        .units
        .iter()
        .find(|row| row.unit == late_hull)
        .expect("its row on the live record");
    assert!(row.arrived, "it joined the running field");
    assert!(
        opening.parties.iter().all(|party| party.party != row.party),
        "a party of its own, lettered next: {:?}",
        live.parties
    );
    assert!(live.parties.iter().any(|party| party.party == row.party));
    // (2) The kernel joins it at the next pulse, beside the pad: B's sensors hold a second
    // carrier there.
    assert!(
        until(&mut rig, &mut rx, &mut lines, 10, |rig, _| near_pad(rig)
            == 2)
        .await,
        "B's sensors hold a second carrier beside the pad: {:?}",
        frame(&rig, b).map(|frame| frame
            .contacts
            .iter()
            .map(|contact| (contact.designator, contact.cell))
            .collect::<Vec<_>>())
    );
    // (3) Its arrival is a row in the presence picture.
    assert!(
        until(&mut rig, &mut rx, &mut lines, 10, |rig, _| {
            tacsim::surface::tick::presence_book(rig.tw.world())
                .and_then(|book| book.sites.get(&site))
                .is_some_and(|flights| flights.iter().any(|flight| flight.tv == late_hull))
        })
        .await,
        "the arrival is a row in the presence picture"
    );
    assert!(task(&rig).is_some(), "the field runs on");

    // (4) No launch goes down from it while the field runs (Q-CF-17).
    let refused = run(&mut rig.tw, late, "+tv/1 launch go");
    assert!(refused.contains("is being fought on"), "{refused}");

    // (5) Nobody fires: the field demotes, and its black box holds the join, sealed mid-field.
    // The demotion checks the arrival in with the field's own rows.
    assert!(
        until(&mut rig, &mut rx, &mut lines, 60, |rig, _| {
            task(rig).is_none()
                && site_record(rig).is_some_and(|record| record.open_field().is_none())
        })
        .await,
        "the field demotes after its quiet interval"
    );
    let sealed: Vec<(ObjId, i64)> = lines
        .iter()
        .filter_map(|line| match &line.kind {
            tacsim::arena::ledger::EventKind::MindSealed { unit, .. } => Some((*unit, line.t_ms)),
            _ => None,
        })
        .collect();
    assert!(
        sealed
            .iter()
            .any(|(unit, at)| *unit == late_hull && *at > 0),
        "the kernel joined it mid-field: {sealed:?}"
    );
    assert!(
        !mush_sim::deployed::is_deployed(rig.tw.world(), late_hull),
        "the arrival is checked in at the demotion"
    );
}

/// A field of two (doc 36a §11 row 5's cap) on `field_host`'s tempo.
fn capped(cfg: &mut TacsimConfig) {
    tune(cfg);
    cfg.field.max_units = 2;
}

/// `field_host`'s tempo with a field that stays up a while: the kernel also refuses a join for
/// real while a mind's think is in flight at the encode (GC.1b's deferred root fix), and the host
/// owes such an arrival again at the next pulse, so the field must outlast the retry.
fn lasting(cfg: &mut TacsimConfig) {
    tune(cfg);
    cfg.field.disengage_s = 600;
}

/// [`pump`], with the kernel's answer to `refused`'s join turned into a refusal on its way to
/// the bridge (doc 36a §11 row 2's one stand-in: production's kernel refuses a join on a scripted
/// seal, which no command makes, or when a mind's think is in flight at the join's encode, a
/// wall-clock race). A kernel that refuses never takes the unit in, so the stand-in also takes it
/// back out of the kernel (a lift-off withdrawal) and swallows that answer: from the refusal on,
/// world and kernel stand as production's would.
fn pump_refusing(
    rig: &mut Rig,
    rx: &mut Receiver<ArenaOut>,
    lines: &mut Vec<LedgerEvent>,
    refused: ObjId,
) -> usize {
    let Some(tasks) = rig
        .tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .map(|registry| registry.tasks().clone())
    else {
        return 0;
    };
    let mut staged = 0;
    while let Ok(message) = rx.try_recv() {
        if let ArenaOut::Ledger { chunk, .. } = &message
            && let Ok(text) = std::str::from_utf8(&chunk.bytes)
        {
            lines.extend(
                text.lines()
                    .filter_map(|line| serde_json::from_str::<LedgerEvent>(line).ok()),
            );
        }
        let message = match message {
            ArenaOut::Joined { match_id, unit, .. } if unit == refused => {
                tasks
                    .offer(
                        match_id,
                        tacsim::arena_task::ArenaMsg::Withdraw { unit, ramp: None },
                    )
                    .expect("the kernel takes the withdrawal");
                ArenaOut::Joined {
                    match_id,
                    unit,
                    outcome: Err("test: the field refused it".to_owned()),
                }
            }
            ArenaOut::Withdrawn { unit, .. } if unit == refused => continue,
            other => other,
        };
        if !tasks.stage(message) {
            break;
        }
        staged += 1;
    }
    if staged > 0 {
        rig.tw.apply(mush_core::WorldCommand::ModuleAsyncDone {
            module: smol_str::SmolStr::new_static(tacsim::arena_bridge::MODULE),
            actor: GOD,
            correlation: 0,
            payload: tacsim::arena_bridge::EGRESS_PAYLOAD.to_owned(),
        });
        tasks.stamp_ledger_writes(1);
        tasks.ack_ledger_upto(1);
    }
    staged
}

/// [`until`], pumped through [`pump_refusing`].
async fn until_refusing(
    rig: &mut Rig,
    rx: &mut Receiver<ArenaOut>,
    lines: &mut Vec<LedgerEvent>,
    refused: ObjId,
    secs: u64,
    mut done: impl FnMut(&mut Rig, &[LedgerEvent]) -> bool,
) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    let mut beat = 0_u32;
    while std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        pump_refusing(rig, rx, lines, refused);
        beat += 1;
        if beat.is_multiple_of(8) {
            let _ = second(rig);
        }
        if done(rig, lines) {
            return true;
        }
    }
    false
}

/// **The bearing a presence row names `to` by from `from`**: the octant between their own poses
/// on the site's ground, as `+field`'s row resolution reads them (a carrier settled on the pad
/// stands at its domain's passive origin, not at the pad's cell).
fn bearing_to(rig: &Rig, from: ObjId, to: ObjId) -> &'static str {
    let cell = |unit: ObjId| {
        mush_sim::pose_of(rig.tw.world(), unit)
            .map(|(_, at)| tacsim::surface::excursion::site_cell(at.x_cm, at.y_cm))
            .expect("on the ground")
    };
    let (to, from) = (cell(to), cell(from));
    tacsim::surface::address::Octant::of(to.0 - from.0, to.1 - from.1)
        .expect("apart")
        .abbrev()
}

/// The party letter `hull`'s row on `live` carries.
fn letter_of(live: &FieldLive, hull: ObjId) -> Option<PartyId> {
    live.units
        .iter()
        .find(|row| row.unit == hull)
        .map(|row| row.party)
}

/// Whether `hull` is settled on Ahuna Flank's pad.
fn settled(rig: &Rig, hull: ObjId) -> bool {
    matches!(
        tacsim::port::ship::berth_of(rig.tw.world(), hull),
        tacsim::port::ship::Berth::Docked { map, .. } if map.entry.as_str() == crate::driving_rig::AHUNA
    )
}

/// **Doc 36a §11 row 2: party letters after a refused join** (GC.1b). Three more Dray-Ps are
/// under way to Ahuna Flank when CF.3's field opens there. The first two settle on the pad
/// together and the host letters each a party of its own; the kernel's answer for the lower-
/// lettered of them is a refusal, which takes its row and its party off the live record — a
/// middle party gone. The third settles after that: its party must take the lowest letter free,
/// the one the refusal gave back, never a letter in use (the old `nth(len)` reused the last
/// party's). The live record passes its own check throughout, and a restart resumes the field
/// with both arrivals in it under the same letters.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn gc1b_a_refused_middle_party_never_lends_its_letter_to_the_next_arrival() {
    let mut rig = stocked_tuned(lasting);
    // No rig goes out: a rig's mind could hold a think in flight when the kernel encodes the
    // joins, and the kernel's own refusal of that race is wall-clock timing, not this story.
    let (a, a_row, _) = on_pad(&mut rig, 1);
    let carrier = ObjId(a_row.hull);
    let (b, _) = prospector(&mut rig, "Other", 2, "Tally Moth");
    let hull_b = armed(&mut rig, "Grey Hound", (120, 128), 90_000);
    seat(&mut rig, hull_b, b);
    let bearing = bearing_to(&rig, hull_b, carrier);
    let (late, late_row, _, late_ticket) = en_route(&mut rig, "Late", 3, "Late Moth");
    let (last, last_row, _, last_ticket) = en_route(&mut rig, "Last", 4, "Last Moth");
    let (third, third_row, _, third_ticket) = en_route(&mut rig, "Third", 5, "Third Moth");
    let (late_hull, last_hull, third_hull) = (
        ObjId(late_row.hull),
        ObjId(last_row.hull),
        ObjId(third_row.hull),
    );
    let arrival = |ticket: &Ticket| ticket.plan.depart_ic + ticket.plan.total_s;
    assert!(
        arrival(&late_ticket) <= arrival(&last_ticket)
            && arrival(&last_ticket) < arrival(&third_ticket),
        "they settle in filing order"
    );
    warm(&mut rig);
    let mut rx = egress(&mut rig);
    let mut lines = Vec::new();
    for _ in 0..3 {
        let _ = second(&mut rig);
    }
    let (out, queued) = command(&mut rig, b, &format!("+field/fire {bearing}"));
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    land(&mut rig, &queued[0]).expect("the promotion lands");
    assert!(
        until(&mut rig, &mut rx, &mut lines, 10, |rig, _| task(rig)
            .is_some())
        .await,
        "the field opens"
    );
    let site = cid(crate::driving_rig::AHUNA);
    let opening = tacsim::field::live_on(rig.tw.world(), &site).expect("the live record");
    assert!(opening.check().is_ok(), "{:?}", opening.parties);

    // (1) The first two settle together, and the host letters both before any answer is pumped.
    let _ = to_ic(&mut rig, arrival(&last_ticket));
    for _ in 0..12 {
        if tacsim::field::live_on(rig.tw.world(), &site).is_some_and(|live| {
            letter_of(&live, late_hull).is_some() && letter_of(&live, last_hull).is_some()
        }) {
            break;
        }
        let _ = second(&mut rig);
    }
    assert!(
        settled(&rig, late_hull) && settled(&rig, last_hull) && !settled(&rig, third_hull),
        "the first two are on the pad, the third still under way"
    );
    let lettered = tacsim::field::live_on(rig.tw.world(), &site).expect("the live record");
    let late_letter = letter_of(&lettered, late_hull).expect("the first arrival is lettered");
    let last_letter = letter_of(&lettered, last_hull).expect("the second arrival is lettered");
    assert_ne!(late_letter, last_letter, "{:?}", lettered.parties);
    assert!(lettered.check().is_ok(), "{:?}", lettered.parties);
    assert!(
        opening
            .parties
            .iter()
            .all(|party| party.party != late_letter && party.party != last_letter),
        "both lettered past the opening's: {:?}",
        lettered.parties
    );

    // (2) The lower-lettered of the two is refused: its row and its party leave the record, the
    // other joins. The refused hull stands outside the field.
    let (refused, kept, freed) = if late_letter < last_letter {
        (late_hull, last_hull, late_letter)
    } else {
        (last_hull, late_hull, last_letter)
    };
    assert!(
        until_refusing(&mut rig, &mut rx, &mut lines, refused, 20, |rig, _| {
            tacsim::field::live_on(rig.tw.world(), &site).is_some_and(|live| {
                live.units.iter().all(|row| row.unit != refused)
                    && live.units.iter().any(|row| row.unit == kept && row.arrived)
            })
        })
        .await,
        "the refusal takes the first arrival off the record and the second joins"
    );
    let after = tacsim::field::live_on(rig.tw.world(), &site).expect("the live record");
    assert!(
        after.parties.iter().all(|party| party.party != freed),
        "the refused party is gone: {:?}",
        after.parties
    );
    assert!(after.check().is_ok(), "{:?}", after.parties);
    assert!(
        tacsim::field::field_of_unit(rig.tw.world(), refused).is_none(),
        "the refused hull stands outside the field"
    );
    assert!(
        !settled(&rig, third_hull),
        "the third is still under way when the refusal lands"
    );
    let kept_letter = letter_of(&after, kept).expect("the kept arrival's letter");

    // (3) The third settles into the gap: the host letters it the lowest free letter, the one the
    // refusal gave back, the moment it takes it in. (The kernel may still refuse it for real while
    // a mind's think is in flight at the encode, GC.1b's deferred root fix; the letter is the
    // host's either way, and a refused hull is not asked again while this field runs.)
    let _ = to_ic(&mut rig, arrival(&third_ticket));
    // Read before any egress is pumped: the kernel's answer lands at the next pump.
    let mut lettered = None;
    for _ in 0..10 {
        lettered = tacsim::field::live_on(rig.tw.world(), &site)
            .filter(|live| letter_of(live, third_hull).is_some());
        if lettered.is_some() {
            break;
        }
        let _ = second(&mut rig);
    }
    let taken = lettered.expect("the host takes the third arrival in");
    let third_letter = letter_of(&taken, third_hull).expect("the third is lettered");
    assert_eq!(
        third_letter, freed,
        "the lowest free letter, never one in use: {:?}",
        taken.parties
    );
    assert_ne!(third_letter, kept_letter);
    assert_eq!(letter_of(&taken, kept), Some(kept_letter));
    assert!(taken.check().is_ok(), "{:?}", taken.parties);
    // The kernel's answer: joined, or refused and off the record.
    assert!(
        until_refusing(&mut rig, &mut rx, &mut lines, refused, 30, |rig, _| {
            tacsim::field::live_on(rig.tw.world(), &site).is_some_and(|live| {
                live.units
                    .iter()
                    .all(|row| row.unit != third_hull || row.arrived)
            })
        })
        .await,
        "the kernel answers the third's join"
    );
    let joined = tacsim::field::live_on(rig.tw.world(), &site).expect("the live record");
    let third_in = letter_of(&joined, third_hull);
    assert!(
        third_in.is_none_or(|letter| letter == freed),
        "{:?}",
        joined.parties
    );
    assert_eq!(letter_of(&joined, kept), Some(kept_letter));
    assert!(joined.check().is_ok(), "{:?}", joined.parties);
    assert!(task(&rig).is_some(), "the field runs on");

    // (4) A restart resumes the field with its arrivals under their letters.
    let field = joined.field.clone();
    let mut rig = crash(rig, &mut rx, &[a, b, late, last, third]).await;
    let mut rx = egress(&mut rig);
    assert!(
        until(&mut rig, &mut rx, &mut lines, 20, |rig, _| task(rig)
            .is_some())
        .await,
        "the field resumes"
    );
    let resumed =
        tacsim::field::live_on(rig.tw.world(), &site).expect("the live record after the restart");
    assert_eq!(
        resumed.field, field,
        "the same field, resumed: never voided and reopened"
    );
    assert!(resumed.check().is_ok(), "{:?}", resumed.parties);
    assert_eq!(letter_of(&resumed, kept), Some(kept_letter));
    assert_eq!(letter_of(&resumed, third_hull), third_in);
    assert!(resumed.units.iter().all(|row| row.unit != refused));
    assert_eq!(
        tacsim::field::field_of_unit(rig.tw.world(), kept),
        Some(site),
        "the kept arrival is in the resumed field"
    );
}

/// **Doc 36a §11 row 5: the cap at the ramp** (Q-CF-21). With `[tacsim.field].max_units` at
/// two, A's Dray-P stands on Ahuna Flank's pad and B's armed hull on its ground: two units the
/// next promotion would admit, a field's full complement. A's launch is refused at the ramp with
/// CF.2's line, where the launch half once counted only the rigs out (none) and let a third unit
/// down for the field to refuse at the opening. The site still promotes: B's fire at the carrier
/// opens a field of exactly those two.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gc_review_the_ramp_refuses_a_launch_the_field_could_not_hold() {
    let mut rig = stocked_tuned(capped);
    let (a, a_row, a_tvs) = on_pad(&mut rig, 1);
    let carrier = ObjId(a_row.hull);
    let (b, _) = prospector(&mut rig, "Other", 2, "Tally Moth");
    let hull_b = armed(&mut rig, "Grey Hound", (120, 128), 90_000);
    seat(&mut rig, hull_b, b);
    let bearing = bearing_to(&rig, hull_b, carrier);
    warm(&mut rig);
    let mut rx = egress(&mut rig);
    let mut lines = Vec::new();
    for _ in 0..3 {
        let _ = second(&mut rig);
    }

    // (1) The ramp refuses the third unit on the ground, naming the field's cap.
    let refused = run(&mut rig.tw, a, "+tv/1 launch go");
    assert!(refused.contains("has 2 rigs out already"), "{refused}");
    assert!(
        crate::driving_rig::record_of(&mut rig.tw, a_tvs[0])
            .is_none_or(|record| record.excursion.is_none()),
        "no excursion opened"
    );

    // (2) The site still promotes, with the two it holds.
    let (out, queued) = command(&mut rig, b, &format!("+field/fire {bearing}"));
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    land(&mut rig, &queued[0]).expect("the promotion lands");
    assert!(
        until(&mut rig, &mut rx, &mut lines, 10, |rig, _| task(rig)
            .is_some())
        .await,
        "the field opens"
    );
    let live = tacsim::field::live_on(rig.tw.world(), &cid(crate::driving_rig::AHUNA))
        .expect("the live record");
    let mut units: Vec<ObjId> = live.units.iter().map(|row| row.unit).collect();
    units.sort_unstable();
    let mut expected = vec![carrier, hull_b];
    expected.sort_unstable();
    assert_eq!(units, expected, "the field holds the carrier and the hull");
}

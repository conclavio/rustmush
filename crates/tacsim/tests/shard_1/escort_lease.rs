//! **Doc 32 cut CF.6c — the armed rig lease** (G-5, doc 32 §12), over the composed world
//! (`driving`'s keyed rig on Ahuna Flank), with the arena's egress pumped by hand as
//! `field_host` pumps it.
//!
//! The cut's gate, through the production command paths:
//!
//! 1. a First Captain leases Occator's escort line tank at the Exchange (`+tv/lease escort`),
//!    under the escort clearance, and takes the operational loan (`+tv/1/permissions go`);
//! 2. they sail to Ahuna Flank, where a miner's prospector works the ground, board the hull in
//!    the bay (`+tv/1 board`, `+focus`, `@station/handoff`), launch it down the ramp and drive it
//!    out beside the rig;
//! 3. an intruder's armed hull stands to the east; the escort fires on it (`+field/fire E`), the
//!    site promotes, and from the line cockpit (`CONTACTS`, `LOCK`, `FIRE`, `STATUS`) their fire
//!    lands and the intruder's lands on them;
//! 4. they withdraw to the ramp (`+tv/1 recover go`, CF.5's exit), the hull comes into the bay
//!    and never launches while the site is promoted (Q-CF-17); its damage persists into its unit
//!    record (CF.3); they sail home and return the lease.
//!
//! The second test is the loss: a leased escort wrecked in a field is written off at its return.
//!
//! The intruder is a fixture hull with a second player seated (CF.2's `armed`/`seat`), so this
//! story never reads the raider content or the doctrine CF.6b changes.

use mush_core::ObjId;
use tacsim::arena::ledger::{EventKind, LedgerEvent};

use crate::driving::{
    RAMP, Rig, act_go, at_counter, command, file_and_launch, finish, land, launched, on_ground_at,
    on_pad_as, on_pad_with, run, staff_hand, stocked_tuned, to_cell, to_ic, warm,
};
use crate::field_host::{
    armed, egress, fragile, frame, seat, second, site_record, task, tune, until, weapon_of,
};

/// The escort's own rig: the lessee's only live record, slot 1.
fn escort_rig(rig: &Rig, pilot: ObjId) -> ObjId {
    let store = rig
        .tw
        .world()
        .module_state::<tacsim::TacsimStore>()
        .expect("the layer");
    let live: Vec<_> = store
        .tv_leases_of(pilot)
        .filter(|record| record.live())
        .collect();
    assert_eq!(live.len(), 1, "one live lease: {live:?}");
    live[0].tv
}

/// **Lease the escort at Occator's counter**: the preview, then the signature, then the loan.
fn lease_escort(rig: &mut Rig, pilot: ObjId) {
    at_counter(rig, pilot);
    let preview = run(&mut rig.tw, pilot, "+tv/lease escort");
    assert!(
        preview.contains("Occator leases you OWP-ES-01")
            && preview.contains("2500.00cr")
            && preview.contains("15000.00cr"),
        "the escort's terms: {preview}"
    );
    // Doc 32 §17: the preview names the command that signs it, never the prospector's.
    assert!(
        preview.contains("+tv/lease escort go signs it.") && !preview.contains("+tv/lease go"),
        "the escort's preview names its own signature: {preview}"
    );
    let nothing = run(&mut rig.tw, pilot, "+tv/lease tank");
    assert!(
        nothing.contains("Occator leases no tank") && nothing.contains("+tv/lease escort"),
        "{nothing}"
    );
    let signed = run(&mut rig.tw, pilot, "+tv/lease escort go");
    assert!(
        signed.contains("Occator Water & Propellant leases you OWP-ES-01"),
        "{signed}"
    );
    // The escort clearance: one armed hull at a time.
    let again = run(&mut rig.tw, pilot, "+tv/lease escort");
    assert!(
        again.contains("one armed hull at a time"),
        "a second armed lease is refused: {again}"
    );
    // The operational loan is the lessor's own Org submission. It is formed here, by the
    // player's own command; landing it needs the production composition's owner preimage reads,
    // which this harness lacks, so mush-server's `dt_preparation_plans` lands it and
    // `board_escort` stands in for its effect.
    let loan = run(&mut rig.tw, pilot, "+tv/1/permissions go");
    assert!(
        loan.contains("recording the four operational loans"),
        "{loan}"
    );
    let mut queued = Vec::new();
    while let Some(submit) = rig.tw.engine_mut().take_authoritative_submit() {
        queued.push(*submit);
    }
    assert!(
        queued
            .iter()
            .any(|submit| submit.action.key().id.as_str() == "unitgrant_issue"),
        "the loan is submitted"
    );
}

/// **Board the leased hull from the carrier's hold** and take its controls: the bay, the cab,
/// the focus and the handoff — the player's own commands.
fn board_escort(rig: &mut Rig, pilot: ObjId, tv: ObjId) {
    // From the airlock to the hold, then the internal bay access.
    for step in ["inner", "hold"] {
        let _ = run(&mut rig.tw, pilot, step);
    }
    let bay = run(&mut rig.tw, pilot, "+tv/bay in");
    assert!(bay.contains("entered vehicle bay"), "{bay}");
    // Without the landed loan the board refuses on OperateCrew, as production does; staff then
    // make the board's two writes (the cab, the helm billet) exactly as `+tv/1 board` makes them.
    let refused = run(&mut rig.tw, pilot, "+tv/1 board");
    assert!(refused.contains("OperateCrew"), "{refused}");
    let cab = tacsim::port::vehicle::rig_row(rig.tw.world(), tv)
        .and_then(|(_, row)| row.rooms.iter().find(|(id, _)| id.as_str() == "cab"))
        .map(|(_, room)| ObjId(*room))
        .expect("the escort's cab");
    staff_hand(rig, |ctx| {
        mush_sim::place_interior(ctx, pilot, cab, None).expect("into the cab");
        ctx.actor = pilot;
        let station = mush_sim::StationId::new("pilot").expect("the console");
        mush_sim::commands::assign_station(ctx, tv, &station, pilot).expect("the helm billet");
    });
    let channels = rig
        .tw
        .world()
        .module_state::<mush_sim::SpatialState>()
        .and_then(|state| state.stations.get(&tv))
        .and_then(|host| host.stations.values().next())
        .map(|spec| spec.channels.len())
        .expect("the escort's console");
    assert_eq!(
        channels, 3,
        "the escort's one console carries helm, sensors and fire"
    );
    let focus = run(&mut rig.tw, pilot, &format!("+focus #{}/pilot", tv.0));
    assert!(!focus.contains("Usage"), "{focus}");
    let handoff = run(
        &mut rig.tw,
        pilot,
        &format!("@station/handoff #{}/pilot", tv.0),
    );
    let _ = handoff;
    for _ in 0..3 {
        let _ = second(rig);
    }
}

/// Hits in `lines` fired by `shooter` that struck `target`.
fn hits(lines: &[LedgerEvent], shooter: ObjId, target: ObjId) -> usize {
    lines
        .iter()
        .filter(|line| {
            matches!(&line.kind, EventKind::Hit { unit, target: struck, .. }
                if *unit == shooter && *struck == target)
        })
        .count()
}

/// Whether `unit` is under a field's mark.
fn in_field(rig: &Rig, unit: ObjId) -> bool {
    mush_sim::deployed::mark_of(rig.tw.world(), unit).is_some_and(|mark| mark.field.is_some())
}

/// **The contact nearest a cell** on `pilot`'s cockpit frame, within two cells.
fn contact_near(
    rig: &Rig,
    pilot: ObjId,
    (cx, cy): (u16, u16),
) -> Option<tacsim::arena::perception::FrameContact> {
    frame(rig, pilot).and_then(|frame| {
        frame
            .contacts
            .iter()
            .find(|contact| {
                contact
                    .cell
                    .is_some_and(|(x, y)| x.abs_diff(cx) <= 2 && y.abs_diff(cy) <= 2)
            })
            .cloned()
    })
}

/// **The escort and the intruder on Ahuna Flank**, the escort launched and driven out beside a
/// working prospector; answers (escort pilot, escort hull, intruder pilot, intruder hull).
fn stand_up(rig: &mut Rig) -> (ObjId, ObjId, ObjId, ObjId) {
    // A miner's prospector works the ground the escort is there for.
    let (miner, _, miner_tvs) = on_pad_as(rig, "Miner", 2, "Tally Moth", 1);
    let worker = miner_tvs[0];
    launched(rig, miner, worker);
    to_cell(rig, worker, (140, 120));
    // (1) The escort's lease, signed at Ceres before the hull sails.
    let (escort, _, tvs) = on_pad_with(rig, "Pilot", 1, "Salt Moth", lease_escort);
    assert_eq!(tvs.len(), 1, "the escort is the hull's one rig");
    let hull = escort_rig(rig, escort);
    assert_eq!(tvs[0], hull);
    // (2) Aboard it in the bay, down the ramp, out beside the rig.
    board_escort(rig, escort, hull);
    launched(rig, escort, hull);
    assert!(on_ground_at(&rig.tw, hull, RAMP));
    let drive = act_go(rig, escort, "+tv/1 drive 136,122 go");
    assert!(!drive.is_empty(), "{drive}");
    let _ = finish(rig, hull, escort);
    // The intruder: a second player's armed hull standing to the east.
    let (intruder, _, _) = on_pad_as(rig, "Intruder", 3, "Lone Moth", 0);
    let hostile = armed(rig, "Grey Hound", (146, 122), 270_000);
    seat(rig, hostile, intruder);
    warm(rig);
    (escort, hull, intruder, hostile)
}

/// **CF.6c's gate** (doc 32 §14): lease, fly out, fight from the line cockpit, withdraw, return.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn cf6c_a_leased_escort_flies_out_fights_withdraws_and_goes_back() {
    // The field stays up while the escort drives home under fire's quiet.
    let mut rig = stocked_tuned(|cfg| {
        tune(cfg);
        cfg.field.disengage_s = 90;
    });
    let (escort, hull, intruder, hostile) = stand_up(&mut rig);
    let before = mush_tactics::module::unit_of(rig.tw.world(), hull)
        .expect("the escort's unit")
        .clone();
    let mut rx = egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();
    for _ in 0..3 {
        let _ = second(&mut rig);
    }

    // (3) The escort fires on the intruder: the site promotes and both hulls are admitted.
    let (out, queued) = command(&mut rig, escort, "+field/fire E");
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    land(&mut rig, &queued[0]).expect("the promotion lands");
    assert!(
        until(&mut rig, &mut rx, &mut lines, 15, |rig, _| task(rig)
            .is_some())
        .await,
        "the field's task opens"
    );
    for unit in [hull, hostile] {
        assert!(in_field(&rig, unit), "#{} is in the field", unit.0);
    }
    // The line cockpit, read from the lessee's own seat.
    assert!(
        until(&mut rig, &mut rx, &mut lines, 20, |rig, _| frame(
            rig, escort
        )
        .is_some())
        .await,
        "the escort's cockpit frame arrives"
    );
    let contacts = run(&mut rig.tw, escort, "CONTACTS");
    assert!(!contacts.trim().is_empty(), "CONTACTS answers: {contacts}");
    let status = run(&mut rig.tw, escort, "STATUS");
    assert!(!status.trim().is_empty(), "STATUS answers: {status}");
    // Fire both ways until the escort's rounds land and the intruder's land on it.
    let gun = weapon_of(&rig, hull);
    let back = weapon_of(&rig, hostile);
    let exchanged = until(&mut rig, &mut rx, &mut lines, 120, |rig, lines| {
        for (pilot, at, weapon) in [
            (escort, (146_u16, 122_u16), gun.as_str()),
            (intruder, (136, 122), back.as_str()),
        ] {
            if let Some(contact) = contact_near(rig, pilot, at)
                && frame(rig, pilot).is_some_and(|frame| frame.lock != Some(contact.id))
            {
                let _ = run(&mut rig.tw, pilot, &format!("LOCK {}", contact.designator));
            }
            let _ = run(&mut rig.tw, pilot, &format!("FIRE {weapon}"));
        }
        hits(lines, hull, hostile) > 0 && hits(lines, hostile, hull) > 0
    })
    .await;
    assert!(
        exchanged,
        "the escort's fire lands and the intruder's lands on it: escort hits {}, intruder hits {}; \
         escort hears {}",
        hits(&lines, hull, hostile),
        hits(&lines, hostile, hull),
        run(&mut rig.tw, escort, "STATUS")
    );
    let status = run(&mut rig.tw, escort, "STATUS");
    assert!(!status.trim().is_empty(), "{status}");

    // (4) Withdraw to the ramp: the pilot drives there from the cockpit, then takes CF.5's exit
    // (`+tv/1 recover go`) and the recover into the bay.
    let _ = run(&mut rig.tw, escort, &format!("FIRE {gun} OFF"));
    let drive = run(&mut rig.tw, escort, "DRIVE 128,124");
    assert!(task(&rig).is_some(), "the field still runs: {drive}");
    let beside = until(&mut rig, &mut rx, &mut lines, 120, |rig, _| {
        frame(rig, escort)
            .and_then(|frame| frame.cell)
            .is_some_and(|(x, y)| x.abs_diff(128) <= 1 && y.abs_diff(124) <= 1)
    })
    .await;
    assert!(
        beside,
        "the escort drives to the ramp ({drive}): {:?}",
        frame(&rig, escort).map(|frame| (frame.cell, frame.immobile, frame.motor))
    );
    let asked = run(&mut rig.tw, escort, "+tv/1 recover go");
    assert!(asked.contains("makes for the ramp"), "{asked}");
    let exited = until(&mut rig, &mut rx, &mut lines, 60, |rig, _| {
        !in_field(rig, hull) && site_record(rig).is_some_and(|record| record.exited(hull))
    })
    .await;
    assert!(
        exited,
        "the escort leaves the field at the ramp: {}",
        run(&mut rig.tw, escort, "+tv/1")
    );
    let home = until(&mut rig, &mut rx, &mut lines, 60, |rig, _| {
        let due = {
            let h = crate::driving::handles(&rig.tw);
            crate::driving::typed(
                &mut rig.tw,
                &h.families.excursions,
                &tacsim::surface::excursion::RigKey(hull),
            )
            .and_then(|record| record.act.map(|act| act.due_ic))
        };
        if let Some(due) = due {
            to_ic(rig, due);
        }
        matches!(
            crate::driving::parent(&rig.tw, hull),
            mush_sim::SpatialParent::Interior { .. }
        )
    })
    .await;
    assert!(
        home,
        "the escort is back in the bay: {}",
        run(&mut rig.tw, escort, "+tv/1")
    );
    // Q-CF-17: it never launches while the site is promoted.
    if site_record(&mut rig).is_some_and(|record| record.open_field().is_some()) {
        let refused = run(&mut rig.tw, escort, "+tv/1 launch go");
        assert!(refused.contains("is being fought"), "{refused}");
    }
    // CF.3: the damage it took persists into its unit record.
    let after = mush_tactics::module::unit_of(rig.tw.world(), hull)
        .expect("the escort's unit")
        .clone();
    assert_ne!(after, before, "the escort's damage persists into its unit");

    // Home: the field demotes, the lessee steps into the bay, the carrier lifts for Ceres and the
    // lease goes back at the counter, the deposit released.
    assert!(
        until(&mut rig, &mut rx, &mut lines, 180, |rig, _| {
            task(rig).is_none()
                && site_record(rig).is_some_and(|record| record.open_field().is_none())
        })
        .await,
        "the field demotes after its quiet interval"
    );
    leave_rig(&mut rig, escort);
    let carrier = carrier_of(&rig, escort);
    let t = file_and_launch(&mut rig, escort, carrier, "+ship/lift ceres");
    to_ic(&mut rig, t.plan.depart_ic + t.plan.total_s);
    // Docked at Ceres: out of the bay, through the hull and its lock, to the counter.
    let out = run(&mut rig.tw, escort, "+tv/bay out");
    assert!(out.contains("Carrier #"), "{out}");
    for step in ["passage", "airlock"] {
        let _ = run(&mut rig.tw, escort, step);
    }
    let ashore = run(&mut rig.tw, escort, "+ship/leave");
    assert!(!ashore.trim().is_empty(), "{ashore}");
    at_counter(&mut rig, escort);
    let preview = run(&mut rig.tw, escort, "+tv/1 lease return");
    assert!(preview.contains("OWP-ES-01"), "{preview}");
    let returned = run(&mut rig.tw, escort, "+tv/1 lease return go");
    assert!(
        returned.contains("OWP-ES-01") && returned.contains("15000.00cr"),
        "the escort goes back and the deposit is released: {returned}"
    );
    let store = rig
        .tw
        .world()
        .module_state::<tacsim::TacsimStore>()
        .expect("the layer");
    let record = store
        .tv_leases_of(escort)
        .find(|record| record.tv == hull)
        .expect("the escort's record");
    assert!(!record.live() && !record.written_off, "{record:?}");
}

/// **Step out of the leased hull into the bay** (`+tv/1 leave`, repeated once the control
/// release settles).
fn leave_rig(rig: &mut Rig, pilot: ObjId) {
    for _ in 0..4 {
        let said = run(&mut rig.tw, pilot, "+tv/1 leave");
        if said.contains("You step into the vehicle bay") {
            return;
        }
        for _ in 0..2 {
            let _ = second(rig);
        }
    }
    panic!("the lessee could not step out of the rig");
}

/// The lessee's chartered hull.
fn carrier_of(rig: &Rig, pilot: ObjId) -> ObjId {
    tacsim::port::live_charter(rig.tw.world(), pilot)
        .expect("a charter")
        .hull
}

/// **The loss** (doc 32 §14's write-off rule): a leased escort wrecked in a field strands its
/// lessee; extracted to Ceres, they write it off at the counter — the lease ends, the deposit is
/// Occator's, the wreck stays where it fell, and the escort clearance bars the next armed lease.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn cf6c_a_leased_escort_wrecked_in_a_field_is_written_off() {
    let mut rig = stocked_tuned(tune);
    let (escort, hull, intruder, hostile) = stand_up(&mut rig);
    fragile(&mut rig, hull);
    let mut rx = egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();
    for _ in 0..3 {
        let _ = second(&mut rig);
    }
    let (out, queued) = command(&mut rig, intruder, "+field/fire W");
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    land(&mut rig, &queued[0]).expect("the promotion lands");
    assert!(
        until(&mut rig, &mut rx, &mut lines, 15, |rig, _| task(rig)
            .is_some())
        .await,
        "the field's task opens"
    );
    let back = weapon_of(&rig, hostile);
    let wrecked = until(&mut rig, &mut rx, &mut lines, 150, |rig, _| {
        if let Some(contact) = contact_near(rig, intruder, (136, 122))
            && frame(rig, intruder).is_some_and(|frame| frame.lock != Some(contact.id))
        {
            let _ = run(
                &mut rig.tw,
                intruder,
                &format!("LOCK {}", contact.designator),
            );
        }
        let _ = run(&mut rig.tw, intruder, &format!("FIRE {back}"));
        frame(rig, escort).is_some_and(|frame| frame.wreck)
    })
    .await;
    assert!(wrecked, "the escort is wrecked");
    assert!(
        until(&mut rig, &mut rx, &mut lines, 60, |rig, _| {
            task(rig).is_none()
                && site_record(rig).is_some_and(|record| record.open_field().is_none())
        })
        .await,
        "the field demotes after its quiet interval"
    );
    assert!(tacsim::port::tv::wrecked(rig.tw.world(), hull), "a wreck");
    let fell = mush_sim::pose_of(rig.tw.world(), hull).map(|(_, pose)| (pose.x_cm, pose.y_cm));
    // Stranded in the wreck, the lessee is extracted to a safe port.
    let asked = run(&mut rig.tw, escort, "+sortie/extract");
    assert!(asked.contains("Extraction requested"), "{asked}");
    let due = tacsim::field::crew_of(rig.tw.world(), escort)
        .and_then(|crew| crew.extraction.as_ref().map(|coming| coming.due_ic))
        .expect("the extraction's due instant");
    to_ic(&mut rig, due);
    assert!(
        until(&mut rig, &mut rx, &mut lines, 30, |rig, _| {
            tacsim::field::crew_of(rig.tw.world(), escort).is_some_and(|crew| {
                matches!(crew.location, tacsim::field::CrewLocation::Port { .. })
            })
        })
        .await,
        "the lessee is at port"
    );
    // At the counter: the write-off.
    at_counter(&mut rig, escort);
    let preview = run(&mut rig.tw, escort, "+tv/1 lease return");
    assert!(preview.contains("is a wreck"), "{preview}");
    let done = run(&mut rig.tw, escort, "+tv/1 lease return go");
    assert!(done.contains("writes OWP-ES-01 off"), "{done}");
    let record = rig
        .tw
        .world()
        .module_state::<tacsim::TacsimStore>()
        .expect("the layer")
        .tv_leases_of(escort)
        .find(|record| record.tv == hull)
        .cloned()
        .expect("the escort's record");
    assert!(!record.live() && record.written_off, "{record:?}");
    assert_eq!(
        mush_sim::pose_of(rig.tw.world(), hull).map(|(_, pose)| (pose.x_cm, pose.y_cm)),
        fell,
        "the wreck stays where it fell"
    );
    // The wreck's excursion no longer counts against any cap, and the escort clearance bars the
    // next armed lease for a while.
    assert!(tacsim::port::tv::written_off(rig.tw.world(), hull));
    let barred = run(&mut rig.tw, escort, "+tv/lease escort");
    assert!(barred.contains("was written off"), "{barred}");
}

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
use mush_org::unit_authority::{UnitCapability, UnitGrantee};
use tacsim::arena::ledger::{EventKind, LedgerEvent};

use crate::driving_rig::{
    RAMP, Rig, act_go, at_counter, command, file_and_launch, finish, land, land_composed, launched,
    on_ground_at, on_pad_as, on_pad_with, pilot, run, stocked_tuned, teleport, to_cell, to_ic,
    typed, warm,
};
use crate::field_host_rig::{
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
    // The operational loan is the lessor's own Org submission, formed by the player's own
    // command and landed on the world task's composed path (`driving::land_composed`, doc 36
    // cut GC.0's hook), as production lands it: the grant then stands in Org's registry and the
    // board below passes on it.
    let loan = run(&mut rig.tw, pilot, "+tv/1/permissions go");
    assert!(
        loan.contains("recording the four operational loans"),
        "{loan}"
    );
    let grants = loan_submissions(rig);
    assert!(!grants.is_empty(), "the loan is submitted");
    for grant in &grants {
        land_composed(rig, grant).expect("the loan lands on the composed path");
    }
}

/// **The Org unit grants queued** (`unitgrant_issue`), taken off the engine's queue; the Org's
/// own submissions are left out by `driving::drain`, so they are taken here.
fn loan_submissions(rig: &mut Rig) -> Vec<mush_core::AuthoritativeSubmit> {
    let mut queued = Vec::new();
    while let Some(submit) = rig.tw.engine_mut().take_authoritative_submit() {
        queued.push(*submit);
    }
    queued
        .into_iter()
        .filter(|submit| submit.action.key().id.as_str() == "unitgrant_issue")
        .collect()
}

/// **Board the leased hull from the carrier's hold** and take its controls: the bay, the cab,
/// the focus and the handoff — the player's own commands, under the landed loan.
fn board_escort(rig: &mut Rig, pilot: ObjId, tv: ObjId) {
    // From the airlock to the hold, then the internal bay access.
    for step in ["inner", "hold"] {
        let _ = run(&mut rig.tw, pilot, step);
    }
    let bay = run(&mut rig.tw, pilot, "+tv/bay in");
    assert!(bay.contains("entered vehicle bay"), "{bay}");
    // The board reads the OperateCrew loan through Org's own authority read and makes its two
    // writes, the cab and the helm billet.
    let boarded = run(&mut rig.tw, pilot, "+tv/1 board");
    assert!(
        boarded.contains("You are at rig 1's"),
        "the lessee boards under the loan: {boarded}"
    );
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

/// The rounds `shooter` fired, at anyone.
fn fired(lines: &[LedgerEvent], shooter: ObjId) -> usize {
    lines
        .iter()
        .filter(|line| matches!(&line.kind, EventKind::Fired { unit, .. } if *unit == shooter))
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
    // Fire both ways.
    let gun = weapon_of(&rig, hull);
    let back = weapon_of(&rig, hostile);
    // The exchange runs until the escort's own frame shows it struck: the story's point is the
    // withdrawal under fire, not a duel between two line tanks, which the field's dice settle in
    // about 30 s of arena time. The frame, not the ledger: the black box's lines reach the host
    // in batches, long after the arena has moved on (doc 40 FR.5's cut note).
    let struck = |rig: &Rig| {
        frame(rig, escort).is_some_and(|frame| {
            frame.sections.iter().any(|section| {
                section
                    .facets
                    .iter()
                    .any(|facet| facet.armor < facet.armor_max)
                    || section.internals.is_some_and(|(now, max)| now < max)
            })
        })
    };
    let exchanged = until(&mut rig, &mut rx, &mut lines, 120, |rig, _| {
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
        struck(rig)
    })
    .await;
    let exchange = |lines: &[LedgerEvent]| {
        format!(
            "escort shots {}, escort hits {}, intruder hits {}, FIELD_SEED {:#x}",
            fired(lines, hull),
            hits(lines, hull, hostile),
            hits(lines, hostile, hull),
            crate::driving_rig::FIELD_SEED
        )
    };
    assert!(
        exchanged,
        "the intruder's fire lands on the escort: {}; escort hears {}",
        exchange(&lines),
        run(&mut rig.tw, escort, "STATUS")
    );
    let status = run(&mut rig.tw, escort, "STATUS");
    assert!(!status.trim().is_empty(), "{status}");

    // (4) Withdraw to the ramp: both cease fire, the pilot drives there from the cockpit, then
    // takes CF.5's exit (`+tv/1 recover go`) and the recover into the bay.
    let _ = run(&mut rig.tw, escort, &format!("FIRE {gun} OFF"));
    let _ = run(&mut rig.tw, intruder, &format!("FIRE {back} OFF"));
    let drive = run(&mut rig.tw, escort, "DRIVE 128,124");
    assert!(
        task(&rig).is_some(),
        "the field still runs: {drive} ({})",
        exchange(&lines)
    );
    let beside = until(&mut rig, &mut rx, &mut lines, 120, |rig, _| {
        frame(rig, escort)
            .and_then(|frame| frame.cell)
            .is_some_and(|(x, y)| x.abs_diff(128) <= 1 && y.abs_diff(124) <= 1)
    })
    .await;
    assert!(
        beside,
        "the escort drives to the ramp ({drive}): {:?}; the exchange: {}",
        frame(&rig, escort).map(|frame| (frame.cell, frame.immobile, frame.motor)),
        exchange(&lines)
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
            let h = crate::driving_rig::handles(&rig.tw);
            crate::driving_rig::typed(
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
            crate::driving_rig::parent(&rig.tw, hull),
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
    // The black box has caught up by the close: the escort's own rounds were fired and landed.
    assert!(
        fired(&lines, hull) > 0 && hits(&lines, hull, hostile) > 0,
        "the escort fired and its rounds landed: {}",
        exchange(&lines)
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

/// **Doc 36 cut GC.0's harness hook** (Q-36-49): the Org unit grant lands in tacsim's world
/// harness. The operational loan `+tv/1/permissions go` submits is refused on the plain prepare,
/// which hands the provider no owner reads, and lands on the composed one
/// (`driving::land_composed`), so a harness story can carry a DT activation's grants. Landed,
/// the grant is on Org's registry, the counter reads it as allowed now, and the lessee's
/// `+tv/1 board` passes on it (doc 36a lane 4).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gc0_the_harness_lands_the_org_unit_grant() {
    let mut rig = stocked_tuned(tune);
    // The loan is typed and landed at the counter, where it is offered, as `lease_escort` types
    // it. The Org's own submissions, which `driving::drain` leaves out, are taken as they queue.
    let (pilot, _, tvs) = on_pad_with(&mut rig, "Pilot", 1, "Salt Moth", |rig, pilot| {
        at_counter(rig, pilot);
        let signed = run(&mut rig.tw, pilot, "+tv/lease escort go");
        assert!(signed.contains("leases you OWP-ES-01"), "{signed}");
        let loan = run(&mut rig.tw, pilot, "+tv/1/permissions go");
        let grants = loan_submissions(rig);
        assert!(!grants.is_empty(), "the loan is submitted: {loan}");
        let plain = land(rig, &grants[0]);
        assert!(
            plain
                .as_ref()
                .is_err_and(|error| error.contains("unit-grant request refused")),
            "the plain prepare refuses the grant: {plain:?}"
        );
        for grant in &grants {
            land_composed(rig, grant).expect("the composed prepare lands the grant");
        }
        // The record: Org's unit grant registry holds the lessee's OperateCrew loan, and the
        // counter's own read says it is allowed now.
        let registry = rig
            .tw
            .world()
            .module_state::<mush_org::unit_authority::runtime::UnitAuthorityHandles>()
            .cloned()
            .expect("the unit authority's handles")
            .registry;
        let grants = typed(&mut rig.tw, &registry, &()).expect("the grant registry");
        assert!(
            grants
                .grants
                .values()
                .any(|grant| grant.grantee == UnitGrantee::Character(pilot)
                    && grant.capability == UnitCapability::OperateCrew),
            "the lessee's OperateCrew loan is on the registry: {:?}",
            grants.grants
        );
        let shown = run(&mut rig.tw, pilot, "+tv/1/permissions");
        assert!(
            shown.contains("allowed now"),
            "the counter reads the loan as current: {shown}"
        );
    });
    // Landed, the loan lets the lessee board the hull in the bay by the production command.
    board_escort(&mut rig, pilot, tvs[0]);
}

/// **Doc 36a lane 3, P1-6**: a delegated OperateCrew grant can be used. The lessee signs the
/// escort and lands Occator's loans at the counter (GC.0's hook), then delegates operate/crew to
/// a crewmate within the delegable loan (`@unitgrant/issue … parent=<loan>`); the crewmate,
/// standing in the carrier's vehicle bay, boards the lessee's rig with `+tv/1 board` and is put
/// at its console, and the lessee stays the lease's principal. Before the fix the board listed
/// only the actor's own leases, so a held grant could never be used.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lane3_a_delegate_with_operate_crew_boards_the_lessees_rig() {
    let mut rig = stocked_tuned(tune);
    let mut loan: Option<u64> = None;
    let (lessee, _, tvs) = on_pad_with(&mut rig, "Pilot", 1, "Salt Moth", |rig, pilot| {
        at_counter(rig, pilot);
        let signed = run(&mut rig.tw, pilot, "+tv/lease escort go");
        assert!(signed.contains("leases you OWP-ES-01"), "{signed}");
        let _ = run(&mut rig.tw, pilot, "+tv/1/permissions go");
        let mut queued = Vec::new();
        while let Some(submit) = rig.tw.engine_mut().take_authoritative_submit() {
            queued.push(*submit);
        }
        let grants: Vec<_> = queued
            .iter()
            .filter(|submit| submit.action.key().id.as_str() == "unitgrant_issue")
            .collect();
        assert!(!grants.is_empty(), "the loans are submitted");
        for grant in grants {
            land_composed(rig, grant).expect("the composed prepare lands the loan");
        }
        // The loans stand; the operate/crew one is delegable within the lease.
        let loans = run(&mut rig.tw, pilot, "+tv/1/permissions");
        let line = loans
            .lines()
            .find(|line| line.contains("operate_crew"))
            .unwrap_or_else(|| panic!("the operate/crew loan is listed: {loans}"));
        assert!(
            line.contains("allowed now") && line.contains("delegable within this lease"),
            "{loans}"
        );
        loan = line
            .split("loan ")
            .nth(1)
            .and_then(|rest| rest.split(':').next())
            .and_then(|n| n.trim().parse().ok());
    });
    let hull = tvs[0];
    let loan = loan.expect("the loan's number");

    // The crewmate stands in the carrier's vehicle bay beside the lessee.
    for step in ["inner", "hold"] {
        let _ = run(&mut rig.tw, lessee, step);
    }
    let bay = run(&mut rig.tw, lessee, "+tv/bay in");
    assert!(bay.contains("entered vehicle bay"), "{bay}");
    let bay_room = rig
        .tw
        .world()
        .get(lessee)
        .and_then(|o| o.location)
        .expect("the bay");
    let mate = pilot(&mut rig, "Mate", 4);
    teleport(&mut rig.tw, mate, bay_room);
    // Without the delegation the crewmate has no rig to board.
    let refused = run(&mut rig.tw, mate, "+tv/1 board");
    assert!(refused.contains("rental rig slots"), "{refused}");

    // The lessee delegates operate/crew within the loan; the grant lands.
    let issued = run(
        &mut rig.tw,
        lessee,
        &format!(
            "@unitgrant/issue #{}=#{} operate_crew parent={loan}",
            hull.0, mate.0
        ),
    );
    let mut queued = Vec::new();
    while let Some(submit) = rig.tw.engine_mut().take_authoritative_submit() {
        queued.push(*submit);
    }
    let grants: Vec<_> = queued
        .iter()
        .filter(|submit| submit.action.key().id.as_str() == "unitgrant_issue")
        .collect();
    assert_eq!(grants.len(), 1, "one delegation is submitted: {issued}");
    land_composed(&mut rig, grants[0]).expect("the composed prepare lands the delegation");

    // The crewmate boards the lessee's rig and is put at its console.
    let boarded = run(&mut rig.tw, mate, "+tv/1 board");
    assert!(
        boarded.contains("console"),
        "the delegate is put at the rig's console: {boarded}"
    );
    assert!(
        tacsim::port::vehicle::rig_row(rig.tw.world(), hull)
            .and_then(|(_, row)| row.rooms.iter().find(|(id, _)| id.as_str() == "cab"))
            .is_some_and(
                |(_, room)| rig.tw.world().get(mate).and_then(|o| o.location) == Some(ObjId(*room))
            ),
        "the delegate stands in the rig's cab"
    );
    // The lessee is still the lease's principal.
    let lease = rig
        .tw
        .world()
        .module_state::<tacsim::TacsimStore>()
        .and_then(|store| store.tv_lease_on(hull).cloned())
        .expect("the live lease");
    assert_eq!(lease.pilot, lessee, "the lessee stays the principal");
}

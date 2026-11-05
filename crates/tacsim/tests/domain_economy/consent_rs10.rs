//! **Doc 41 cut RS.10 — the landholder consent step** (§4.4 "On its own account"; the owner's
//! rulings of 2026-11-04), over the composed lane at 256 (Ahuna Flank) and 512 (Urvara Reach):
//!
//! Occator's spare rig lies wrecked on a block the pilot holds the Registry's claim on. After 56
//! IC days Tycho Mutual's sweep reaches her, and on owned ground it first asks: the registry's
//! notice goes to the landholder, and the sweep passes the block over while it waits.
//!
//! - **Silence**: 28 IC days with no answer is consent; the house recovers her, and the pin at
//!   the attach reads the landholder's authorization (a lawful recovery off owned ground).
//! - **Refusal**: the landholder's `+salvage/refuse <site>/<block>=<who> go` blocks the house on
//!   that block for 28 IC days; once it lapses the house asks again.
//!
//! **What is a fixture.** The rig's sections destroyed and its row on the site's index as a wreck,
//! by staff hand (GC.5's fixture); the pilot's block claim seeded as a typed record (GC.6's);
//! IC days skipped. Every act of Tycho's and the landholder's is the production path.

use mush_core::ObjId;
use mush_org::PrincipalId;
use tacsim::surface::site_record::SiteRecord;

use crate::driving_rig::{
    Rig, cid, fire, land_quietly, on_pad_as_at, port_map, ramp_at, run, seed_typed, stocked_tuned,
    to_cell_on,
};
use crate::field_host_rig::{second, set_section, site_record_at, tune};
use crate::site_record_rig::{staff, staff_member};

const DAY: i64 = 86_400;

/// Short tow stages and a ten-minute claim window.
fn quick(cfg: &mut tacsim::TacsimConfig) {
    tune(cfg);
    cfg.field.idle_demote_s = 3_600;
    cfg.recovery.inspect_s = 30;
    cfg.recovery.attach_s = 30;
    cfg.recovery.load_s = 60;
    cfg.recovery.claim_window_ic_s = 600;
}

/// Move IC on to `target`, then fire a few times.
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

/// A few seconds of the world.
fn seconds(rig: &mut Rig, n: u32) {
    for _ in 0..n {
        let t = rig.tw.world().now().saturating_add(1);
        rig.tw.set_now(t);
        let _ = second(rig);
    }
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

/// `holder`'s claim on `letter` of `entry`, seeded (GC.6's fixture).
fn seed_block(rig: &mut Rig, entry: &str, letter: char, holder: ObjId) {
    let place = port_map(&rig.tw, entry)
        .site
        .and_then(|site| site.blocks.get(letter.to_string().as_str()).copied())
        .map(ObjId)
        .expect("the block's place");
    let now = rig.tw.world().world_ic_now().at;
    let mut tenure = tacsim::surface::claims::BlockTenure::new(cid(entry), letter, place);
    tenure.claim = Some(tacsim::surface::claims::BlockClaim {
        holder: PrincipalId::Object(holder),
        account: rig
            .tw
            .world()
            .module_state::<mush_sim::AccountOracle>()
            .and_then(|oracle| oracle.account_of(holder)),
        epoch: 1,
        opened_ic: None,
        registered_ic: now,
        term_start_ic: now,
        term_end_ic: now + 365 * DAY,
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

/// Whether a custody row on `entry` holds `hull`.
fn in_custody(rig: &mut Rig, entry: &str, hull: ObjId) -> bool {
    site_record_at(rig, entry)
        .is_some_and(|record| record.custody.iter().any(|row| row.hull == hull))
}

/// The dispatcher's ask on `letter`, if any.
fn ask_of(
    record: &SiteRecord,
    letter: char,
    dispatcher: ObjId,
) -> Option<tacsim::surface::claim::ConsentAsk> {
    record
        .consent_asks
        .iter()
        .find(|ask| ask.letter == letter && ask.asker == dispatcher)
        .cloned()
}

/// The world: the pilot (the landholder), Occator's spare rig wrecked on the pilot's block, the
/// house's dispatcher and lifter, and IC skipped past the sweep's bar until the house asks.
struct Story {
    rig: Rig,
    landholder: ObjId,
    wreck: ObjId,
    letter: char,
    dispatcher: ObjId,
    lifter: ObjId,
}

fn asked(venue: &str, entry: &str, side: u16) -> Story {
    let mut rig = stocked_tuned(quick);
    let (landholder, _, tvs) = on_pad_as_at(&mut rig, ("Pilot", 1, "Salt Moth"), (venue, entry), 1);
    let schema = mush_tactics::module::unit_of(rig.tw.world(), tvs[0])
        .expect("a rig")
        .schema()
        .clone();
    let wreck = rig
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
    to_cell_on(&mut rig, entry, wreck, at);
    set_section(&mut rig, wreck, "chassis", Some("destroyed"), Some(0));
    set_section(&mut rig, wreck, "boom", Some("destroyed"), Some(0));
    let s = staff_member(&mut rig);
    staff(
        &mut rig,
        s,
        &format!(
            "@tacsim/site {entry} leave #{} wreck {},{}",
            wreck.0, at.0, at.1
        ),
    );
    let letter = letter_of(side, at);
    seed_block(&mut rig, entry, letter, landholder);
    let ceres = cid("ceres_port");
    for _ in 0..60 {
        let (_, queued) = fire(&mut rig);
        land_quietly(&mut rig, &queued);
        if tacsim::port::house::dispatchers(rig.tw.world()).len() == 1 {
            break;
        }
    }
    let (lifter, _) = tacsim::port::house::hulls(rig.tw.world(), &ceres).expect("Tycho's house");
    let dispatcher = tacsim::port::house::dispatcher(rig.tw.world(), &ceres).expect("dispatcher");
    // On their own block the landholder needs no ask.
    let own = run(
        &mut rig.tw,
        landholder,
        &format!("+salvage/ask {entry}/{letter}"),
    );
    assert!(own.contains("already recover"), "{own}");
    let now = rig.tw.world().world_ic_now().at;
    skip_and_fire(
        &mut rig,
        now + (tacsim::port::house_sweep::SWEEP_IC_DAYS + 1) * DAY,
    );
    seconds(&mut rig, 30);
    let record = site_record_at(&mut rig, entry).expect("the record");
    let ask = ask_of(&record, letter, dispatcher).expect("the house asked the landholder");
    assert_eq!(ask.landholder, landholder);
    assert_eq!(ask.refused_ic, None);
    // The block waits on its landholder: the house leaves her be.
    seconds(&mut rig, 120);
    assert!(
        !in_custody(&mut rig, entry, wreck),
        "nothing is recovered while the landholder may answer"
    );
    assert!(matches!(
        tacsim::port::ship::berth_of(rig.tw.world(), lifter),
        tacsim::port::ship::Berth::Docked { .. }
    ));
    Story {
        rig,
        landholder,
        wreck,
        letter,
        dispatcher,
        lifter,
    }
}

/// **Silence is consent** on `(venue, entry)`.
fn silence(venue: &str, entry: &str, side: u16) {
    let Story {
        mut rig,
        landholder,
        wreck,
        lifter,
        ..
    } = asked(venue, entry, side);
    let now = rig.tw.world().world_ic_now().at;
    skip_and_fire(
        &mut rig,
        now + (tacsim::surface::claim::CONSENT_IC_DAYS + 1) * DAY,
    );
    let mut pin = None;
    for _ in 0..20_000 {
        if let Some(found) = site_record_at(&mut rig, entry).and_then(|record| {
            record
                .custody
                .iter()
                .find(|row| row.hull == wreck)
                .and_then(|row| row.pin.clone())
        }) {
            pin = Some(found);
            break;
        }
        let step = if matches!(
            tacsim::port::ship::berth_of(rig.tw.world(), lifter),
            tacsim::port::ship::Berth::UnderWay
        ) {
            300
        } else {
            1
        };
        let t = rig.tw.world().now().saturating_add(step);
        rig.tw.set_now(t);
        let _ = second(&mut rig);
    }
    let pin = pin.expect("the house recovered her after 28 IC days of silence");
    assert_eq!(pin.landholder(), Some(landholder), "{pin:?}");
    assert!(
        pin.landholder_authorized,
        "silence is the landholder's consent: {pin:?}"
    );
}

/// **RS.10's silence on Ahuna Flank (256).**
#[test]
fn rs10_silence_is_consent_on_ahuna_flank() {
    silence("ahuna", "ahuna_port", 256);
}

/// **RS.10's silence on Urvara Reach (512).**
#[test]
fn rs10_silence_is_consent_on_urvara_reach() {
    silence("urvara", "urvara_port", 512);
}

/// **A refusal blocks for 28 IC days** (Ahuna Flank, 256), then the house asks again.
#[test]
fn rs10_a_refusal_blocks_the_sweep_for_28_ic_days() {
    let Story {
        mut rig,
        landholder,
        wreck,
        letter,
        dispatcher,
        ..
    } = asked("ahuna", "ahuna_port", 256);
    let entry = "ahuna_port";
    let preview = run(
        &mut rig.tw,
        landholder,
        &format!("+salvage/refuse {entry}/{letter}=#{}", dispatcher.0),
    );
    assert!(preview.contains("Add go"), "{preview}");
    let refused = run(
        &mut rig.tw,
        landholder,
        &format!("+salvage/refuse {entry}/{letter}=#{} go", dispatcher.0),
    );
    assert!(refused.contains("records your refusal"), "{refused}");
    seconds(&mut rig, 5);
    let record = site_record_at(&mut rig, entry).expect("the record");
    let at = ask_of(&record, letter, dispatcher)
        .and_then(|ask| ask.refused_ic)
        .expect("the refusal is on the record");
    // Past the ask's own 28 days, inside the refusal's: still nothing.
    skip_and_fire(&mut rig, at + 20 * DAY);
    seconds(&mut rig, 120);
    assert!(
        !in_custody(&mut rig, entry, wreck),
        "the refusal blocks the house"
    );
    // The refusal lapses: the house asks again.
    skip_and_fire(
        &mut rig,
        at + (tacsim::surface::claim::CONSENT_IC_DAYS + 1) * DAY,
    );
    seconds(&mut rig, 30);
    let record = site_record_at(&mut rig, entry).expect("the record");
    let again = ask_of(&record, letter, dispatcher).expect("an ask");
    assert_eq!(again.refused_ic, None, "a fresh ask: {again:?}");
    assert!(again.asked_ic > at, "{again:?}");
}

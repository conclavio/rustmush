//! **Doc 36 cut GC.13 — crew treatment** (Q-36-45), over the composed world (`driving`'s keyed
//! rig on Ahuna Flank), the arena's egress pumped by hand as `field_host` pumps it.
//!
//! The cut's gate: a pilot incapacitated by a control-section kill is extracted to a port and is
//! fit again after the configured treatment time, which a medical room shortens; one who leaves
//! the port keeps their condition and starts over at the next.

use mush_core::{GOD, ObjId, ObjKind};
use tacsim::TacsimConfig;
use tacsim::arena::ledger::LedgerEvent;
use tacsim::arena_task::ArenaOut;
use tacsim::field::crew::{FIT_AGAIN, Treatment};
use tacsim::field::{CrewCondition, CrewLocation, crew_of};
use tokio::sync::mpsc::Receiver;

use crate::driving::{
    Rig, command, drain, land, on_pad, on_pad_as, run, staff_hand, stocked_tuned, warm,
};
use crate::field_crash::{control_destroyed, hull_of};
use crate::field_host::{armed, egress, seat, second, until};

/// Treatment of an incapacitated character, IC seconds (the gate's compressed time).
const TREAT_INCAPACITATED_S: u32 = 600;
/// The medical room's share, per mille.
const MEDICAL_PERMILLE: u32 = 500;

/// A fast field, a short extraction and a short treatment (`field_crash`'s tune).
fn tune(cfg: &mut TacsimConfig) {
    crate::field_host::tune(cfg);
    cfg.field.extract_s = 60;
    cfg.field.treat_injured_s = 300;
    cfg.field.treat_incapacitated_s = TREAT_INCAPACITATED_S;
    cfg.field.treat_medical_permille = MEDICAL_PERMILLE;
}

/// One world second, as [`second`], answering what `who` heard from it.
fn second_heard(rig: &mut Rig, who: ObjId) -> String {
    let t = rig.tw.world().now().saturating_add(1);
    rig.tw.set_now(t);
    let events = rig.tw.fire_due(t.saturating_mul(1_000));
    let heard = rig.tw.seen_by(&events, who).join("\n");
    for submit in drain(&mut rig.tw) {
        let _ = land(rig, &submit);
    }
    heard
}

/// Up to `secs` world seconds until `done` holds; answers everything `who` heard on the way.
fn heard_until(rig: &mut Rig, who: ObjId, secs: u32, done: impl Fn(&Rig) -> bool) -> String {
    let mut heard = String::new();
    for _ in 0..secs {
        heard.push_str(&second_heard(rig, who));
        heard.push('\n');
        if done(rig) {
            break;
        }
    }
    heard
}

/// **CF-5a's opening** (`field_crash`): B flies a biped from inside it, its head destroyed, is
/// incapacitated in the field's first frames, asks for extraction and lands at a port. `before`
/// runs once the extraction is asked for, before it lands. Answers B, the egress and the ledger.
async fn extracted_incapacitated(
    rig: &mut Rig,
    before: impl FnOnce(&mut Rig),
) -> (ObjId, Receiver<ArenaOut>, Vec<LedgerEvent>) {
    let (a, _, _) = on_pad(rig, 1);
    let (b, _, _) = on_pad_as(rig, "Other", 2, "Tally Moth", 1);
    let hull_a = armed(rig, "Red Hound", (110, 124), 90_000);
    let hull_b = hull_of(
        rig,
        "tacsim.unit.biped_mk1",
        "Grey Strider",
        (114, 124),
        270_000,
    );
    seat(rig, hull_a, a);
    seat(rig, hull_b, b);
    staff_hand(rig, |ctx| {
        ctx.w.move_to(b, hull_b).expect("B climbs into the hull");
    });
    control_destroyed(rig, hull_b);
    warm(rig);
    let mut rx = egress(rig);
    let mut lines = Vec::new();
    for _ in 0..3 {
        let _ = second(rig);
    }
    let (out, queued) = command(rig, a, "+field/fire E");
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    land(rig, &queued[0]).expect("the promotion lands");
    let incapacitated = until(rig, &mut rx, &mut lines, 20, |rig, _| {
        crew_of(rig.tw.world(), b).is_some_and(|crew| {
            crew.condition == CrewCondition::Incapacitated
                && crew.location == CrewLocation::Unit { unit: hull_b }
        })
    })
    .await;
    assert!(
        incapacitated,
        "B is incapacitated by the control-section kill"
    );
    let status = run(&mut rig.tw, b, "+sortie/condition");
    assert!(
        status.contains("Crew: incapacitated, aboard #")
            && status.contains("treatment begins at the next port"),
        "{status}"
    );

    let asked = run(&mut rig.tw, b, "+sortie/extract");
    assert!(asked.contains("Extraction requested"), "{asked}");
    before(rig);
    let due = crew_of(rig.tw.world(), b)
        .and_then(|crew| crew.extraction.as_ref().map(|coming| coming.due_ic))
        .expect("the extraction's due instant");
    while rig.tw.world().world_ic_now().at < due {
        let _ = second(rig);
    }
    let extracted = until(rig, &mut rx, &mut lines, 20, |rig, _| {
        crew_of(rig.tw.world(), b)
            .is_some_and(|crew| matches!(crew.location, CrewLocation::Port { .. }))
    })
    .await;
    assert!(extracted, "B is at port after the extraction delay");
    (b, rx, lines)
}

/// B's treatment under way.
fn treatment(rig: &Rig, b: ObjId) -> Treatment {
    crew_of(rig.tw.world(), b)
        .and_then(|crew| crew.treatment.clone())
        .expect("B is under treatment")
}

/// **GC.13's gate** (doc 36 §7; Q-36-45): B is incapacitated by a control-section kill, extracted
/// to a port with a medical room and under treatment there at the medical room's time. Leaving
/// the port drops the treatment and keeps the condition; back at the port it starts over, and at
/// its due instant B is fit again and told so.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gc13_an_extracted_incapacitated_pilot_is_fit_again_after_treatment() {
    let mut rig = stocked_tuned(tune);
    let (b, _rx, _lines) = extracted_incapacitated(&mut rig, |_| {}).await;
    let port = rig.tw.world().get(b).and_then(|object| object.location);
    let first = treatment(&rig, b);
    assert!(first.medical, "both lane ports have a medical room");
    let medical_s = i64::from(TREAT_INCAPACITATED_S * MEDICAL_PERMILLE / 1_000);
    assert_eq!(
        first.due_ic - first.started_ic,
        medical_s,
        "the medical room's time"
    );
    let status = run(&mut rig.tw, b, "+sortie/condition");
    assert!(
        status.contains("Crew: incapacitated, at ")
            && status.contains("'s medical room, fit again in about 1 IC h"),
        "{status}"
    );

    // Out of the port: the treatment is dropped, the condition kept.
    let away = staff_hand(&mut rig, |ctx| {
        let room = ctx
            .w
            .create(ObjKind::Room, "A Room Off The Map", GOD, None)
            .expect("a room");
        ctx.w.move_to(b, room).expect("B steps out");
        room
    });
    let heard = heard_until(&mut rig, b, 10, |rig| {
        crew_of(rig.tw.world(), b).is_some_and(|crew| crew.treatment.is_none())
    });
    assert!(heard.contains("starts over at the next port"), "{heard}");
    let crew = crew_of(rig.tw.world(), b).cloned().expect("B's record");
    assert_eq!(crew.condition, CrewCondition::Incapacitated, "{crew:?}");
    assert_eq!(crew.treatment, None, "{crew:?}");
    assert_eq!(rig.tw.world().get(b).and_then(|o| o.location), Some(away));

    // Back at the port: the clock starts over, in full.
    let back_at = rig.tw.world().world_ic_now().at;
    staff_hand(&mut rig, |ctx| {
        ctx.w
            .move_to(b, port.expect("the arrival room"))
            .expect("B walks back in");
    });
    let heard = heard_until(&mut rig, b, 10, |rig| {
        crew_of(rig.tw.world(), b).is_some_and(|crew| crew.treatment.is_some())
    });
    assert!(heard.contains("You are under treatment at"), "{heard}");
    let again = treatment(&rig, b);
    assert!(
        again.started_ic >= back_at,
        "{again:?} began after {back_at}"
    );
    assert_eq!(again.due_ic - again.started_ic, medical_s);

    // On to the due instant: fit, and told.
    let mut told = String::new();
    while rig.tw.world().world_ic_now().at < again.due_ic {
        told.push_str(&second_heard(&mut rig, b));
    }
    told.push_str(&heard_until(&mut rig, b, 10, |rig| {
        crew_of(rig.tw.world(), b).is_some_and(|crew| crew.condition == CrewCondition::Fit)
    }));
    assert!(told.contains(FIT_AGAIN), "B hears it: {told:?}");
    let crew = crew_of(rig.tw.world(), b).cloned().expect("B's record");
    assert_eq!(crew.condition, CrewCondition::Fit, "{crew:?}");
    assert_eq!(crew.treatment, None);
    assert!(
        rig.tw.world().world_ic_now().at >= again.due_ic,
        "not before its time"
    );
    let status = run(&mut rig.tw, b, "+sortie/condition");
    assert!(status.starts_with("Crew: fit, at "), "{status}");
}

/// **GC.13, the medical room is faster** (Q-36-45): the same path with every medical room closed
/// before the extraction lands takes the full treatment time, twice the medical room's.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gc13_a_port_without_a_medical_room_treats_slower() {
    let mut rig = stocked_tuned(tune);
    let (b, _rx, _lines) = extracted_incapacitated(&mut rig, |rig| {
        staff_hand(rig, |ctx| {
            let store = ctx
                .w
                .module_state_mut::<mush_industry::IndustryStore>()
                .expect("the industry store");
            let medical: Vec<mush_industry::FacilityRecord> = store
                .facilities()
                .into_iter()
                .filter(|facility| {
                    facility
                        .capabilities()
                        .iter()
                        .any(|word| word.as_str() == tacsim::port::def::MEDICAL_CAPABILITY)
                })
                .cloned()
                .collect();
            assert!(!medical.is_empty(), "the lane opened its medical rooms");
            for facility in medical {
                // Closed: the room keeps its facility, without the capability.
                let closed = mush_industry::FacilityRecord::new(
                    facility.object(),
                    facility.owner().clone(),
                    std::collections::BTreeSet::new(),
                    facility.maximum_slots(),
                )
                .expect("a closed record");
                store.put_facility(closed);
            }
        });
    })
    .await;
    let slow = treatment(&rig, b);
    assert!(!slow.medical, "{slow:?}");
    assert_eq!(
        slow.due_ic - slow.started_ic,
        i64::from(TREAT_INCAPACITATED_S),
        "the full time without a medical room"
    );
    assert!(
        i64::from(TREAT_INCAPACITATED_S * MEDICAL_PERMILLE / 1_000) < slow.due_ic - slow.started_ic,
        "a medical room treats faster"
    );
    let status = run(&mut rig.tw, b, "+sortie/condition");
    assert!(
        status.contains("fit again in about 1 IC h") && !status.contains("medical room"),
        "{status}"
    );
}

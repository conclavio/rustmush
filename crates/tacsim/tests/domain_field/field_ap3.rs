//! **Doc 45 unit AP.3 — the posture record** (Q-45-3, Q-45-4, Q-45-23), over the composed
//! world (`driving`'s keyed rig on Ahuna Flank) with the arena's egress pumped by hand.
//!
//! A unit's posture is its own: one record on the unit's object, `RETURN` until its player
//! sets another. A field that opens around the unit reads it, a restart keeps it, and a
//! demotion leaves it where it was. The story follows one hull through all of that, in a
//! field of three parties (`field_mw7`'s yard: the ward, a raider, a third captain's escort):
//!
//! 1. the ward is set to `GUARD` on the open surface, with no field anywhere;
//! 2. the site promotes around it, and the ward's own frame, which its mind's view is built
//!    from, reads `guard`, while the raider's reads `return`;
//! 3. set to `HOLD` in the running field, its frame reads `hold`;
//! 4. set back to `RETURN` and the server restarted at once, the resumed field reads
//!    `return`, though the snapshot it resumed from was taken on `HOLD`;
//! 5. set to `GUARD` in the resumed field, its frame reads `guard`;
//! 6. nobody fires for a minute, the field demotes, and the record still says `GUARD`.
//!
//! What stands in: AP.3 has no verb (setting a posture as an order is given is doc 45 unit
//! AP.4), so the story writes through the record's one writer, `tacsim::posture::record::set`,
//! as a verb will. What a mind does with the word is AP.6's; that a sealed script reads it and
//! acts on it is pinned at the arena (`domain_arena`, `ap3_a_sealed_script_reads_its_posture`).

use mush_core::ObjId;
use tacsim::arena::ledger::LedgerEvent;
use tacsim::posture::Posture;

use crate::driving_rig::{Rig, crash, on_pad_as, run, stocked_tuned};
use crate::field_host_rig::{armed, egress, frame, seat, site_record, task, tune, until};
use crate::field_mw7::{WARD_AT, Yard, yard_with};

/// The yard's parties stand in range of each other after the promoting shot, so the field
/// never disengages; it demotes on its idle bound, a minute with no shot fired (doc 36 cut
/// GC.2), counted again from a resume.
fn brisk(cfg: &mut tacsim::TacsimConfig) {
    tune(cfg);
    cfg.field.disengage_s = 3_600;
    cfg.field.idle_demote_s = 60;
}

/// Set `unit`'s posture through the record's one writer.
fn set(rig: &mut Rig, unit: ObjId, posture: Posture) {
    let changed = crate::support::with_ctx(&mut rig.tw, |ctx| {
        tacsim::posture::record::set(ctx, unit, posture)
    });
    assert_eq!(changed, Ok(true), "the record moves to {posture}");
}

/// The posture `unit`'s record holds.
fn held(rig: &Rig, unit: ObjId) -> Posture {
    tacsim::posture::record::of(rig.tw.world(), unit)
}

/// The posture `pilot`'s own frame carries.
fn read(rig: &Rig, pilot: ObjId) -> Option<Posture> {
    frame(rig, pilot).map(|frame| frame.posture)
}

/// **The story** (this module's head).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn ap3_a_units_posture_is_its_own_through_a_field_a_restart_and_a_demotion() {
    let mut rig = stocked_tuned(brisk);
    let (pilot, _, _) = on_pad_as(&mut rig, "Pilot", 1, "Salt Moth", 0);
    let ward = armed(&mut rig, "Ward Dog", WARD_AT, 180_000);
    seat(&mut rig, ward, pilot);

    // (1) Every unit holds RETURN until its player says otherwise; the ward is set to GUARD
    // on the open surface.
    assert_eq!(held(&rig, ward), Posture::Return);
    assert!(task(&rig).is_none(), "no field stands yet");
    set(&mut rig, ward, Posture::Guard);
    assert_eq!(held(&rig, ward), Posture::Guard);

    // (2) The site promotes around it: three parties, and the ward's frame reads its record.
    let mut yard = yard_with(rig, pilot, ward).await;
    let (stranger, raider, captain) = (yard.stranger, yard.raider, yard.captain);
    assert!(
        yard.wait(20, |rig, _| read(rig, pilot) == Some(Posture::Guard))
            .await,
        "the field reads the posture the unit came in with: {:?}",
        read(&yard.rig, pilot)
    );
    assert_eq!(
        read(&yard.rig, stranger),
        Some(Posture::Return),
        "the raider's is its own, and nobody set it"
    );
    assert_eq!(held(&yard.rig, raider), Posture::Return);

    // (3) Changed in the running field, the unit reads the new word.
    set(&mut yard.rig, ward, Posture::Hold);
    assert!(
        yard.wait(20, |rig, _| read(rig, pilot) == Some(Posture::Hold))
            .await,
        "a posture set mid-field reaches the unit: {:?}",
        read(&yard.rig, pilot)
    );
    // The field saves every half second: its snapshot now holds HOLD.
    let _ = yard.wait(2, |_, _| false).await;

    // (4) Set back to RETURN, and the server goes down before the field can save again.
    set(&mut yard.rig, ward, Posture::Return);
    let Yard { rig, mut rx, .. } = yard;
    let mut rig = crash(rig, &mut rx, &[pilot, captain, stranger]).await;
    let mut rx = egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();
    assert!(
        until(&mut rig, &mut rx, &mut lines, 30, |rig, _| task(rig)
            .is_some())
        .await,
        "the field resumes"
    );
    assert_eq!(
        held(&rig, ward),
        Posture::Return,
        "the record is the unit's"
    );
    // Focus never survives a boot: the pilot takes the cockpit back.
    let _ = run(&mut rig.tw, pilot, &format!("+focus #{}/cockpit", ward.0));
    assert!(
        until(&mut rig, &mut rx, &mut lines, 30, |rig, _| read(rig, pilot)
            .is_some())
        .await,
        "the resumed seat is sent a frame"
    );
    assert!(
        !until(&mut rig, &mut rx, &mut lines, 4, |rig, _| read(rig, pilot)
            != Some(Posture::Return))
        .await,
        "the resumed field reads the record, not the snapshot's older word: {:?}",
        read(&rig, pilot)
    );

    // (5) And the resumed host tells the resumed field a new word.
    set(&mut rig, ward, Posture::Guard);
    assert!(
        until(&mut rig, &mut rx, &mut lines, 20, |rig, _| read(rig, pilot)
            == Some(Posture::Guard))
        .await,
        "a posture set after the restart reaches the unit: {:?}",
        read(&rig, pilot)
    );

    // (6) Nobody fires for a minute, the field demotes, and the record stands.
    assert!(
        until(&mut rig, &mut rx, &mut lines, 240, |rig, _| {
            task(rig).is_none()
                && site_record(rig).is_some_and(|record| record.open_field().is_none())
        })
        .await,
        "the field demotes"
    );
    assert_eq!(
        held(&rig, ward),
        Posture::Guard,
        "the posture is the unit's after the field is gone"
    );
    assert_eq!(held(&rig, raider), Posture::Return);
    // And it is set on the open surface as it was before any field.
    set(&mut rig, ward, Posture::Hold);
    assert_eq!(held(&rig, ward), Posture::Hold);
}

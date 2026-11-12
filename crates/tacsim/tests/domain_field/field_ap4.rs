//! **Doc 45 unit AP.4 — setting a posture** (Q-45-19), over the composed world (`driving`'s
//! keyed rig on Ahuna Flank), every line typed by a player who is not staff.
//!
//! A posture is set the way an order reaches a unit: from its seat, from aboard its carrier
//! (or the rig's own cab), or over the comms relay, which jamming can stop; never from an OOC
//! room. The unit's status and its carrier's roster show it.
//!
//! 1. `ap4_a_posture_is_set_from_aboard_the_carrier_and_never_from_an_ooc_room`: an
//!    operator with two rigs, aboard her carrier. She reads and sets each rig's, the roster
//!    shows a line under the rig that holds one and none under a rig on `RETURN`, a second
//!    set says nothing changed, and a word that is no posture is refused. From an OOC room
//!    every set is refused and nothing changes; a read still answers.
//! 2. `ap4_a_posture_is_set_from_the_units_seat_in_a_field`: a field of three parties
//!    (`field_mw7`'s yard). The ward's pilot reads and sets the posture from the cockpit; the
//!    record, the hull's own frame and `STATUS` carry it; a word that is no posture is refused.
//! 3. `ap4_from_a_rigs_cab_its_own_posture_is_set_at_once_and_another_rigs_over_the_relay`:
//!    `escort_lease`'s ground, the lessee with a second rig. He is in his armed rig's cab on
//!    open ground, off his carrier (he boarded it in the bay with `+tv/1 board` and drove it
//!    out). His own rig's posture is set at once, interference or not. His order to the rig
//!    in the carrier's bay goes over the relay and arrives; with interference declared where
//!    that rig stands it does not arrive and the rig holds what it held; with it lifted the
//!    same order arrives.
//!
//! What stands in: the interference is declared by a staff character with `@prop/jam`, as
//! doc 15's stories declare it. That a seat in the simulator is refused is pinned in
//! `domain_arena` (`cockpit`), where a practice match runs.

use mush_core::{LIMBO, ObjId};
use tacsim::posture::Posture;
use tacsim::posture::order::{
    POSTURE_JAMMED, POSTURE_KEPT, POSTURE_LINE, POSTURE_NOT_FLOWN_YET, POSTURE_OOC, POSTURE_READ,
    POSTURE_RELAYED, POSTURE_SET, POSTURE_WORDS, line,
};

use crate::driving_rig::{Rig, on_pad_as, run, stocked_tuned};
use crate::field_host_rig::{armed, frame, seat, tune};
use crate::field_mw7::{WARD_AT, quiet, yard_with};
use crate::site_record_rig::staff_member;

/// The posture `unit`'s record holds.
fn held(rig: &Rig, unit: ObjId) -> Posture {
    tacsim::posture::record::of(rig.tw.world(), unit)
}

/// Where `obj` is to the comms relay.
fn place_of(rig: &Rig, obj: ObjId) -> ObjId {
    let world = rig.tw.world();
    let spatial = world
        .module_state::<mush_sim::SpatialState>()
        .expect("the spatial state");
    mush_propagation::context::place_of(spatial, world, obj).expect("it is somewhere")
}

/// **Story 1** (this module's head): the carrier, its roster, and the OOC room.
#[test]
#[allow(clippy::too_many_lines)]
fn ap4_a_posture_is_set_from_aboard_the_carrier_and_never_from_an_ooc_room() {
    let mut rig = stocked_tuned(tune);
    let (operator, _, tvs) = on_pad_as(&mut rig, "Pilot", 1, "Salt Moth", 2);
    let (one, two) = (tvs[0], tvs[1]);

    // (1) Aboard the carrier: the read, the set, the roster.
    assert_eq!(
        run(&mut rig.tw, operator, "+tv/1 posture"),
        line(POSTURE_READ, "Rig 1", Posture::Return)
    );
    let roster = run(&mut rig.tw, operator, "+tv");
    assert!(
        !roster.contains("posture"),
        "a rig on RETURN shows no line: {roster}"
    );
    assert_eq!(
        run(&mut rig.tw, operator, "+tv/1 posture guard"),
        format!(
            "{}\n{POSTURE_NOT_FLOWN_YET}",
            line(POSTURE_SET, "Rig 1", Posture::Guard)
        )
    );
    assert_eq!(
        (held(&rig, one), held(&rig, two)),
        (Posture::Guard, Posture::Return)
    );
    let roster = run(&mut rig.tw, operator, "+tv");
    let shown = POSTURE_LINE.replace("{posture}", "GUARD");
    assert_eq!(
        roster.matches(&shown).count(),
        1,
        "the roster shows rig 1's posture, once: {roster}"
    );
    let first = roster.find("Rig 1:").expect("rig 1's line");
    let second = roster.find("Rig 2:").expect("rig 2's line");
    let at = roster.find(&shown).expect("the posture line");
    assert!(first < at && at < second, "under rig 1: {roster}");
    assert_eq!(
        run(&mut rig.tw, operator, "+tv/1 posture guard"),
        format!(
            "{}\n{POSTURE_NOT_FLOWN_YET}",
            line(POSTURE_KEPT, "Rig 1", Posture::Guard)
        ),
        "a second set changes nothing and says so"
    );
    assert_eq!(
        run(&mut rig.tw, operator, "+tv/1 posture flee"),
        POSTURE_WORDS
    );
    assert_eq!(
        run(&mut rig.tw, operator, "+tv/1 posture return"),
        line(POSTURE_SET, "Rig 1", Posture::Return)
    );
    assert_eq!(held(&rig, one), Posture::Return);

    assert_eq!(
        run(&mut rig.tw, operator, "+tv/2 posture hold"),
        format!(
            "{}\n{POSTURE_NOT_FLOWN_YET}",
            line(POSTURE_SET, "Rig 2", Posture::Hold)
        )
    );

    // (2) From an OOC room every set is refused and nothing changes; the read still answers.
    rig.tw
        .world_mut()
        .move_to(operator, LIMBO)
        .expect("to an OOC room");
    for typed in ["+tv/1 posture return", "+tv/2 posture return"] {
        assert_eq!(run(&mut rig.tw, operator, typed), POSTURE_OOC, "{typed}");
    }
    assert_eq!(
        (held(&rig, one), held(&rig, two)),
        (Posture::Return, Posture::Hold)
    );
    assert_eq!(
        run(&mut rig.tw, operator, "+tv/2 posture"),
        line(POSTURE_READ, "Rig 2", Posture::Hold)
    );
}

/// **Story 2** (this module's head): the seat, in a field of three parties.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ap4_a_posture_is_set_from_the_units_seat_in_a_field() {
    let mut rig = stocked_tuned(quiet);
    let (pilot, _, _) = on_pad_as(&mut rig, "Pilot", 1, "Salt Moth", 0);
    let ward = armed(&mut rig, "Ward Dog", WARD_AT, 180_000);
    seat(&mut rig, ward, pilot);
    let mut yard = yard_with(rig, pilot, ward).await;
    let stranger = yard.stranger;
    let raider = yard.raider;

    // The read, from the seat.
    let read = run(&mut yard.rig.tw, pilot, "MIND POSTURE");
    assert!(
        read.ends_with(&line(POSTURE_READ, "", Posture::Return)),
        "{read}"
    );
    let status = run(&mut yard.rig.tw, pilot, "STATUS");
    assert!(
        !status.contains("posture"),
        "a hull on RETURN shows no line: {status}"
    );

    // The set, from the seat: the record at once, the hull's own frame within a pulse or two.
    let set = run(&mut yard.rig.tw, pilot, "MIND POSTURE GUARD");
    assert!(
        set.contains(&line(POSTURE_SET, "", Posture::Guard)) && set.contains(POSTURE_NOT_FLOWN_YET),
        "{set}"
    );
    assert_eq!(held(&yard.rig, ward), Posture::Guard);
    assert!(
        yard.wait(20, |rig, _| frame(rig, pilot)
            .is_some_and(|frame| frame.posture == Posture::Guard))
            .await,
        "the hull's frame, and so its mind's view, reads the new word"
    );
    let status = run(&mut yard.rig.tw, pilot, "STATUS");
    assert!(
        status.contains(&POSTURE_LINE.replace("{posture}", "GUARD")),
        "the unit's status shows it: {status}"
    );

    // A word that is no posture is refused, and nothing changes; the raider's is its own.
    assert_eq!(
        run(&mut yard.rig.tw, pilot, "MIND POSTURE FLEE"),
        POSTURE_WORDS
    );
    assert_eq!(held(&yard.rig, ward), Posture::Guard);
    assert_eq!(held(&yard.rig, raider), Posture::Return);
    let theirs = run(&mut yard.rig.tw, stranger, "MIND POSTURE HOLD");
    assert!(
        theirs.contains(&line(POSTURE_SET, "", Posture::Hold)),
        "{theirs}"
    );
    assert_eq!(
        (held(&yard.rig, ward), held(&yard.rig, raider)),
        (Posture::Guard, Posture::Hold),
        "each seat sets its own hull's and no other"
    );
}

/// **Story 3** (this module's head): the rig's own cab, and the relay to another rig.
#[test]
#[allow(clippy::too_many_lines)]
fn ap4_from_a_rigs_cab_its_own_posture_is_set_at_once_and_another_rigs_over_the_relay() {
    let mut rig = stocked_tuned(tune);
    // The lessee boarded his armed rig in the bay, took it down the ramp and drove it out
    // beside a working prospector; a prospector of his own is still in his carrier's bay.
    let (lessee, armed_rig, _intruder, _hostile, others) =
        crate::escort_lease::stand_up_with(&mut rig, 1);
    let prospector = others[0];
    assert_eq!(
        (held(&rig, armed_rig), held(&rig, prospector)),
        (Posture::Return, Posture::Return)
    );
    let staff = staff_member(&mut rig);
    let jam = |rig: &mut Rig, unit: ObjId, level: u8| {
        let place = place_of(rig, unit);
        let said = run(
            &mut rig.tw,
            staff,
            &format!("@prop/jam #{}={level}", place.0),
        );
        assert!(said.contains("Interference at"), "{said}");
    };

    // (1) The rig he sits in: set at once. Interference where it stands would stop an order
    // over the relay; his is not one.
    jam(&mut rig, armed_rig, 4);
    assert_eq!(
        run(&mut rig.tw, lessee, "+tv/1 posture hold"),
        format!(
            "{}\n{POSTURE_NOT_FLOWN_YET}",
            line(POSTURE_SET, "Rig 1", Posture::Hold)
        )
    );
    assert_eq!(held(&rig, armed_rig), Posture::Hold);
    jam(&mut rig, armed_rig, 0);

    // (2) His other rig is in the carrier's bay, and he is neither aboard the carrier nor in
    // that rig's cab: his order to it goes over the relay, and arrives.
    assert_eq!(
        run(&mut rig.tw, lessee, "+tv/2 posture guard"),
        format!(
            "{}\n{POSTURE_NOT_FLOWN_YET}",
            line(POSTURE_RELAYED, "Rig 2", Posture::Guard)
        )
    );
    assert_eq!(held(&rig, prospector), Posture::Guard);

    // (3) Interference is declared where that rig stands: the order does not arrive, and the
    // rig holds what it held. The read is his own book and needs no relay.
    jam(&mut rig, prospector, 4);
    assert_eq!(
        run(&mut rig.tw, lessee, "+tv/2 posture hold"),
        line(POSTURE_JAMMED, "Rig 2", Posture::Guard)
    );
    assert_eq!(
        held(&rig, prospector),
        Posture::Guard,
        "a jammed order changes nothing"
    );
    assert_eq!(
        run(&mut rig.tw, lessee, "+tv/2 posture"),
        line(POSTURE_READ, "Rig 2", Posture::Guard)
    );

    // (4) The interference is lifted and the same order arrives.
    jam(&mut rig, prospector, 0);
    assert_eq!(
        run(&mut rig.tw, lessee, "+tv/2 posture hold"),
        format!(
            "{}\n{POSTURE_NOT_FLOWN_YET}",
            line(POSTURE_RELAYED, "Rig 2", Posture::Hold)
        )
    );
    assert_eq!(
        (held(&rig, armed_rig), held(&rig, prospector)),
        (Posture::Hold, Posture::Hold)
    );
}

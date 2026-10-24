//! **A prospecting licence bought through the real verb** (doc 20 cut SM.5): `+tv/lease` now
//! refuses without a licence in force, so every suite whose pilots lease a rig licenses them
//! here first, once, through `+licence/buy go` (never a hand-written record).

use mush_core::ObjId;
use mush_testkit::TestWorld;

/// `+licence/buy go` as `pilot`, from wherever they stand (the verb is accepted from
/// anywhere), asserting the Registry licensed them; what the pilot heard.
pub fn buy_licence(tw: &mut TestWorld, pilot: ObjId) -> String {
    let events = tw.run(pilot, "+licence/buy go");
    let out = tw.seen_by(&events, pilot).join("\n");
    assert!(
        out.contains("licenses you to prospect at its sites until"),
        "the pilot is licensed: {out}"
    );
    out
}

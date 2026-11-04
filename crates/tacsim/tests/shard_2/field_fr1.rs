//! **Doc 40 cut FR.1 — declared stances** (§4.2; Q-38-23, Q-40-11..14, round 2's 40-3), over
//! the composed world with the arena's egress pumped by hand, as `field_gc10.rs` pumps it, once
//! at 256 a side (Ahuna Flank) and once at 512 (Urvara Reach), one script over both.
//!
//! Three co-ops — Concord, Basin and Halo — declare through `+faction/stance`: Concord and Basin
//! Hostile both ways, Concord and Halo Allied both ways, Halo Allied toward Basin alone. Concord's
//! leader is also an open member of Basin, so the two parties share a member and would be Friends
//! by organization (GC.10) but for the stance. Then:
//!
//! - a stance never opens a field (Q-40-13); a `+field/mark` by Concord's leader promotes;
//! - each side's cockpit picture holds the other Hostile from the first pulse it classifies it;
//!   Halo is Concord's Friend both ways (mutual Allied), and Basin is not Halo's (one-sided);
//! - Concord's leader's clear of Basin's party returns to the standing Hostile, not to the
//!   Friend the shared member would make it (40-3); a Friendly mark still replaces the stance;
//! - Halo withdraws its Allied mid-field and nothing in the field changes; the next field reads it;
//! - a fourth captain in Basin lands mid-field and joins under Concord's declared Hostile in the
//!   existing party's book (`carrier::joining`).
//!
//! **What is a fixture.** The armed hulls are made and seated by staff hand (CF.2's fixture), the
//! co-ops are factions made and staffed by `@faction/create` and `@faction/add` (GC.10's
//! precedent: mush-org has no membership verb, so an organization's stance is exercised on the
//! declare path only). Every declaration, mark and clear is a player's command. The two sites are
//! FR.0's `grounds_rig` (the field story's grounds); the hulls' cells are this file's own.

use mush_core::{GOD, ObjId};
use tacsim::arena::PartyId;
use tacsim::arena::party::Relation;
use tacsim::arena::state::Classification;
use tacsim::field::{FieldLive, FieldParty, PartyKey};

use crate::driving_rig::{
    Rig, cid, command, en_route_at, file_and_launch, land, on_pad_as_at, prospector, run,
    staff_hand, stocked_tuned, to_ic, warm,
};
use crate::field_host_rig::{armed_on, egress, frame, seat, second, task, tune, until};
use crate::grounds_rig::{AHUNA_FLANK, Ground, Placed, URVARA_REACH};

/// **Where one run stands**: one of the field story's two grounds (FR.0's `grounds_rig`) and where
/// this story's three armed hulls stand on it. Basin's hull is Concord's nearest neighbour due
/// east, Halo's off to the north or south.
struct Stand {
    /// The site.
    ground: &'static Ground,
    /// Concord's hull.
    concord: Placed,
    /// Basin's hull, east of Concord's.
    basin: Placed,
    /// Halo's hull, north or south of Concord's.
    halo: Placed,
}

/// **Ahuna Flank at 256 a side**: `field_gc10`'s two cells, and Halo south of Concord.
const ON_AHUNA: Stand = Stand {
    ground: &AHUNA_FLANK,
    concord: ((110, 124), 90_000),
    basin: ((114, 124), 270_000),
    halo: ((110, 128), 90_000),
};

/// **Urvara Reach at 512 a side**: the field story's fight row, across the 256 line.
const ON_URVARA: Stand = Stand {
    ground: &URVARA_REACH,
    concord: ((258, 268), 90_000),
    basin: ((262, 268), 270_000),
    halo: ((258, 262), 90_000),
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fr1_declared_stances_at_256() {
    stances_story(&ON_AHUNA).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fr1_declared_stances_at_512() {
    stances_story(&ON_URVARA).await;
}

/// `who` joins `faction` openly at `level` (2 is an officer).
fn join(rig: &mut Rig, faction: &str, who: ObjId, level: u8) {
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@faction/add {faction}=#{}/level:{level}", who.0),
    );
    assert!(
        !out.contains("No such"),
        "#{} joins {faction}: {out}",
        who.0
    );
}

/// The relation `who`'s cockpit picture gives each party it has classified.
fn view(rig: &Rig, who: ObjId) -> Vec<(PartyId, Relation)> {
    frame(rig, who)
        .map(|frame| {
            frame
                .contacts
                .iter()
                .filter_map(|contact| match contact.class {
                    Classification::Classified { party } => Some((party, contact.relation)),
                    Classification::Unknown => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The relation `who`'s picture gives `party`, once classified.
fn relation(rig: &Rig, who: ObjId, party: PartyId) -> Option<Relation> {
    view(rig, who)
        .into_iter()
        .find(|(seen, _)| *seen == party)
        .map(|(_, relation)| relation)
}

/// The party `unit`'s row on the live record carries.
fn party_of(live: &FieldLive, unit: ObjId) -> PartyId {
    live.units
        .iter()
        .find(|row| row.unit == unit)
        .map(|row| row.party)
        .unwrap_or_else(|| panic!("#{} has no row: {:?}", unit.0, live.units))
}

/// The live record's row for `party`.
fn row(live: &FieldLive, party: PartyId) -> &FieldParty {
    live.parties
        .iter()
        .find(|row| row.party == party)
        .unwrap_or_else(|| panic!("no party {party:?}: {:?}", live.parties))
}

/// **Every relation `who`'s picture gives `party` until `done`**, pumping the field; answers
/// whether `done` held and every relation seen on the way.
async fn watch(
    rig: &mut Rig,
    rx: &mut tokio::sync::mpsc::Receiver<tacsim::arena_task::ArenaOut>,
    who: ObjId,
    party: PartyId,
    secs: u64,
    mut done: impl FnMut(Relation) -> bool,
) -> (bool, Vec<Relation>) {
    let mut seen = Vec::new();
    let mut lines = Vec::new();
    let held = until(rig, rx, &mut lines, secs, |rig, _| {
        let now = relation(rig, who, party);
        if let Some(now) = now
            && seen.last() != Some(&now)
        {
            seen.push(now);
        }
        now.is_some_and(&mut done)
    })
    .await;
    (held, seen)
}

/// A `+field/mark` inside the field: one line, nothing submitted.
fn mark(rig: &mut Rig, who: ObjId, line: &str) -> String {
    let (out, queued) = command(rig, who, line);
    assert!(
        queued.is_empty(),
        "a mark in a field submits nothing: {out}"
    );
    out
}

#[allow(clippy::too_many_lines)]
async fn stances_story(at: &Stand) {
    let mut rig = stocked_tuned(tune);
    let site = cid(at.ground.entry);
    // A neutral captain's carrier opens the site's pad; the three co-ops' captains stay in port
    // and fly their hulls from a remote cab, each the leader of one party.
    let (host, host_row, _) = on_pad_as_at(
        &mut rig,
        ("Host", 4, "Quiet Moth"),
        (at.ground.venue, at.ground.entry),
        0,
    );
    // It sails home again, so no mind stands on the ground: a join is refused while a carrier's
    // think is in flight at its encode (doc 36 cut GC.1b's race), which is not this story.
    let _ = file_and_launch(&mut rig, host, ObjId(host_row.hull), "+ship/undock ceres");
    let (a, _) = prospector(&mut rig, "Pilot", 1, "Salt Moth");
    let (b, _) = prospector(&mut rig, "Other", 2, "Tally Moth");
    let (c, _) = prospector(&mut rig, "Third", 3, "Lone Moth");
    let place = |rig: &mut Rig, name: &str, (cell, heading): Placed, pilot: ObjId| {
        let hull = armed_on(rig, at.ground.entry, name, cell, heading);
        seat(rig, hull, pilot);
        hull
    };
    let hull_a = place(&mut rig, "Red Hound", at.concord, a);
    let hull_b = place(&mut rig, "Grey Hound", at.basin, b);
    let hull_c = place(&mut rig, "Pale Hound", at.halo, c);

    // The co-ops, each captain an officer of its own; Concord's captain is also an open member
    // of Basin, the member the two share.
    for name in ["Concord", "Basin", "Halo"] {
        let out = run(&mut rig.tw, GOD, &format!("@faction/create {name}"));
        assert!(out.contains(name), "{out}");
    }
    join(&mut rig, "Concord", a, 2);
    join(&mut rig, "Basin", a, 1);
    join(&mut rig, "Basin", b, 2);
    join(&mut rig, "Halo", c, 2);

    // (1) The declarations, by the co-ops' officers (Q-40-11).
    let said = run(&mut rig.tw, a, "+faction/stance Concord=Basin/hostile");
    assert!(
        said.contains("Concord now stands Hostile toward Basin"),
        "{said}"
    );
    let said = run(&mut rig.tw, b, "+faction/stance Basin=Concord/hostile");
    assert!(
        said.contains("Basin now stands Hostile toward Concord"),
        "{said}"
    );
    let said = run(&mut rig.tw, c, "+faction/stance Halo=Concord/allied");
    assert!(said.contains("Unreciprocated"), "one side only: {said}");
    let said = run(&mut rig.tw, a, "+faction/stance Concord=Halo/allied");
    assert!(
        said.contains("Concord now stands Allied toward Halo") && !said.contains("Unreciprocated"),
        "now mutual: {said}"
    );
    let said = run(&mut rig.tw, c, "+faction/stance Halo=Basin/allied");
    assert!(said.contains("Unreciprocated"), "{said}");
    // A plain member declares nothing; a stranger's name for a side is no side.
    let refused = run(&mut rig.tw, a, "+faction/stance Basin=Halo/hostile");
    assert!(
        refused.contains("You do not declare Basin's stance"),
        "{refused}"
    );
    let refused = run(&mut rig.tw, a, "+faction/stance Concord=Nobody/hostile");
    assert!(refused.contains("No faction or organization"), "{refused}");
    // `+faction` shows the rows.
    let shown = run(&mut rig.tw, b, "+faction Concord");
    assert!(
        shown.contains("Stance toward Basin: Hostile (mutual).")
            && shown.contains("Stance toward Halo: Allied (mutual)."),
        "{shown}"
    );
    let shown = run(&mut rig.tw, c, "+faction Halo");
    assert!(
        shown.contains("Stance toward Basin: Allied (unreciprocated)."),
        "{shown}"
    );
    // An organization's declarer holds `org.stance.declare` (Q-40-11); staff always declare.
    run(&mut rig.tw, GOD, "@org/create Ridge");
    let refused = run(&mut rig.tw, c, "+faction/stance Ridge=Basin/hostile");
    assert!(
        refused.contains("You do not declare Ridge's stance"),
        "{refused}"
    );
    let granted = run(
        &mut rig.tw,
        GOD,
        &format!("@org/grant #{}=Ridge org.stance.declare", c.0),
    );
    let said = run(&mut rig.tw, c, "+faction/stance Ridge=Basin/hostile");
    assert!(
        said.contains("Ridge now stands Hostile toward Basin"),
        "{granted} / {said}"
    );
    let shown = run(&mut rig.tw, b, "+faction Ridge");
    assert!(
        shown.contains("(an organization)") && shown.contains("Stance toward Basin: Hostile."),
        "{shown}"
    );
    let said = run(&mut rig.tw, GOD, "@faction/stance Ridge=Basin/neutral");
    assert!(
        said.contains("Ridge now stands Neutral toward Basin"),
        "{said}"
    );

    // (2) On the ground before any field: Concord and Basin are Friends by the shared member,
    // Concord and Halo by the mutual Allied; Halo and Basin are not; each Hostile is on its
    // party's row (what admission writes).
    let (parties, pairs) = staff_hand(&mut rig, |ctx| {
        tacsim::field::friends_on_ground(&*ctx, &site)
    });
    let letter = |unit: ObjId| {
        parties
            .iter()
            .find(|party| party.key == PartyKey::Unit { unit })
            .map(|party| party.party)
            .unwrap_or_else(|| panic!("#{} has no party: {parties:?}", unit.0))
    };
    let (pa, pb, pc) = (letter(hull_a), letter(hull_b), letter(hull_c));
    let paired = |x: PartyId, y: PartyId| pairs.contains(&(x.min(y), x.max(y)));
    assert!(paired(pa, pb), "the shared member: {pairs:?}");
    assert!(paired(pa, pc), "the mutual Allied: {pairs:?}");
    assert!(!paired(pb, pc), "a one-sided Allied is nothing: {pairs:?}");
    let hostile_of = |party: PartyId| {
        parties
            .iter()
            .find(|row| row.party == party)
            .map(|row| row.stance_hostile.clone())
            .unwrap_or_default()
    };
    assert_eq!(hostile_of(pa), vec![pb], "{parties:?}");
    assert_eq!(hostile_of(pb), vec![pa], "{parties:?}");
    assert!(hostile_of(pc).is_empty(), "{parties:?}");

    // A fourth captain, in Basin, is under way to the site.
    let (d, d_row, _, ticket) = en_route_at(
        &mut rig,
        ("Late", 5, "Late Moth"),
        (at.ground.venue, at.ground.entry),
    );
    let late_hull = ObjId(d_row.hull);
    join(&mut rig, "Basin", d, 2);

    // (3) A stance never opens a field (Q-40-13): seconds pass with two declared enemies on one
    // site, and the ground stays open.
    warm(&mut rig);
    let mut rx = egress(&mut rig);
    for _ in 0..3 {
        let _ = second(&mut rig);
    }
    assert!(task(&rig).is_none(), "a declaration opens nothing");

    // (4) Concord's leader marks Basin's hull, due east: the field opens.
    let (out, queued) = command(&mut rig, a, "+field/mark E=hostile");
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    land(&mut rig, &queued[0]).expect("the promotion lands");
    let mut lines = Vec::new();
    assert!(
        until(&mut rig, &mut rx, &mut lines, 20, |rig, _| task(rig)
            .is_some())
        .await,
        "the field opens"
    );
    let live = tacsim::field::live_on(rig.tw.world(), &site).expect("the live record");
    assert_eq!(
        (
            party_of(&live, hull_a),
            party_of(&live, hull_b),
            party_of(&live, hull_c)
        ),
        (pa, pb, pc),
        "admission letters as the ground read"
    );
    assert_eq!(row(&live, pa).stance_hostile, vec![pb]);
    assert_eq!(row(&live, pb).stance_hostile, vec![pa]);

    // (5) Each picture holds the other Hostile from the first pulse it classifies it — Basin's
    // by the stance alone, over the member it shares.
    let (held, seen) = watch(&mut rig, &mut rx, a, pb, 30, |r| r == Relation::Hostile).await;
    assert!(
        held && seen == [Relation::Hostile],
        "Concord sees Basin: {seen:?}"
    );
    let (held, seen) = watch(&mut rig, &mut rx, b, pa, 30, |r| r == Relation::Hostile).await;
    assert!(
        held && seen == [Relation::Hostile],
        "Basin sees Concord: {seen:?}"
    );
    // Halo is Concord's Friend both ways; Basin is not Halo's.
    let (held, seen) = watch(&mut rig, &mut rx, a, pc, 30, |r| r == Relation::Friend).await;
    assert!(
        held && seen == [Relation::Friend],
        "Concord sees Halo: {seen:?}"
    );
    let (held, seen) = watch(&mut rig, &mut rx, c, pa, 30, |r| r == Relation::Friend).await;
    assert!(
        held && seen == [Relation::Friend],
        "Halo sees Concord: {seen:?}"
    );
    // A friendship is one symmetric pair, so whichever of Halo and Basin classifies the other
    // first tells it: never a Friend (Halo's hull may not see Basin's past the ground between).
    let mut both = Vec::new();
    let one_sided = until(&mut rig, &mut rx, &mut lines, 30, |rig, _| {
        both.extend(relation(rig, c, pb));
        both.extend(relation(rig, b, pc));
        !both.is_empty()
    })
    .await;
    assert!(
        one_sided && !both.contains(&Relation::Friend),
        "a one-sided Allied: Halo and Basin see {both:?}"
    );

    // (6) The clear returns to the stance (40-3), not to the shared member's Friend; a Friendly
    // mark replaces the stance, and a clear of it returns to the stance again.
    let said = mark(&mut rig, a, &format!("+field/mark {}=clear", pb.letter()));
    assert!(said.contains("cleared to the standing Hostile"), "{said}");
    for _ in 0..3 {
        let _ = second(&mut rig);
    }
    let (held, seen) = watch(&mut rig, &mut rx, a, pb, 5, |_| false).await;
    assert!(
        !held && seen == [Relation::Hostile],
        "the clear holds Basin Hostile: {seen:?}"
    );
    let said = mark(
        &mut rig,
        a,
        &format!("+field/mark {}=friendly", pb.letter()),
    );
    assert!(
        said.contains("marks party") && said.contains("Friendly"),
        "{said}"
    );
    let (held, seen) = watch(&mut rig, &mut rx, a, pb, 20, |r| r == Relation::Friend).await;
    assert!(held, "a Friendly mark replaces the stance: {seen:?}");
    let said = mark(&mut rig, a, &format!("+field/mark {}=clear", pb.letter()));
    assert!(said.contains("cleared to the standing Hostile"), "{said}");
    let (held, seen) = watch(&mut rig, &mut rx, a, pb, 20, |r| r == Relation::Hostile).await;
    assert!(held, "and the clear returns to it: {seen:?}");

    // (7) Halo withdraws its Allied mid-field: the running field is unchanged.
    let said = run(&mut rig.tw, c, "+faction/stance Halo=Concord/neutral");
    assert!(
        said.contains("A field already running is unchanged"),
        "{said}"
    );
    let (held, seen) = watch(&mut rig, &mut rx, a, pc, 5, |_| false).await;
    assert!(
        !held && seen == [Relation::Friend],
        "Halo stays Concord's Friend in this field: {seen:?}"
    );

    // (8) Basin's fourth captain lands mid-field and joins in a party of its own, under Concord's
    // declared Hostile in Concord's book.
    let _ = to_ic(&mut rig, ticket.plan.depart_ic + ticket.plan.total_s);
    let joined = until(&mut rig, &mut rx, &mut lines, 60, |rig, _| {
        tacsim::field::live_on(rig.tw.world(), &site)
            .is_some_and(|live| live.units.iter().any(|row| row.unit == late_hull))
    })
    .await;
    assert!(
        joined,
        "the late carrier joins the running field: berth {:?}, owed {:?}, task {:?}, units {:?}",
        tacsim::port::ship::berth_of(rig.tw.world(), late_hull),
        tacsim::field::carrier::owed_arrivals(rig.tw.world(), &site),
        task(&rig),
        tacsim::field::live_on(rig.tw.world(), &site).map(|live| live
            .units
            .iter()
            .map(|row| row.unit)
            .collect::<Vec<_>>())
    );
    let live = tacsim::field::live_on(rig.tw.world(), &site).expect("the live record");
    let pd = party_of(&live, late_hull);
    assert!(![pa, pb, pc].contains(&pd), "a party of its own");
    assert!(
        row(&live, pa).stance_hostile.contains(&pd),
        "Concord's book holds the arrival Hostile: {:?}",
        live.parties
    );
    assert!(
        row(&live, pd).stance_hostile.contains(&pa),
        "and Basin's arrival holds Concord Hostile: {:?}",
        live.parties
    );
    assert!(
        row(&live, pc).stance_hostile.is_empty() && row(&live, pb).stance_hostile == [pa],
        "the join opens only its own pairs: {:?}",
        live.parties
    );
    let (held, seen) = watch(&mut rig, &mut rx, a, pd, 30, |r| r == Relation::Hostile).await;
    assert!(
        held && seen == [Relation::Hostile],
        "Concord's picture holds the arrival Hostile: {seen:?}"
    );

    // (9) Everyone stands down; with no Hostile left the field disengages.
    for (who, of) in [(a, pb), (a, pd), (b, pa), (d, pa)] {
        let said = mark(
            &mut rig,
            who,
            &format!("+field/mark {}=friendly", of.letter()),
        );
        assert!(said.contains("Friendly"), "{said}");
    }
    assert!(
        until(&mut rig, &mut rx, &mut lines, 90, |rig, _| task(rig)
            .is_none())
        .await,
        "the field disengages"
    );
    assert!(tacsim::field::live_on(rig.tw.world(), &site).is_none());

    // (10) The next field reads the table as it stands: Halo is no longer Concord's Friend, and
    // the captain who arrived is admitted under the declared Hostile.
    for _ in 0..3 {
        let _ = second(&mut rig);
    }
    let (out, queued) = command(&mut rig, a, "+field/mark E=hostile");
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    land(&mut rig, &queued[0]).expect("the promotion lands");
    assert!(
        until(&mut rig, &mut rx, &mut lines, 20, |rig, _| task(rig)
            .is_some())
        .await,
        "the second field opens"
    );
    let live = tacsim::field::live_on(rig.tw.world(), &site).expect("the live record");
    let (pa, pc, pd) = (
        party_of(&live, hull_a),
        party_of(&live, hull_c),
        party_of(&live, late_hull),
    );
    assert!(
        row(&live, pa).stance_hostile.contains(&pd),
        "{:?}",
        live.parties
    );
    let (held, seen) = watch(&mut rig, &mut rx, a, pc, 30, |_| true).await;
    assert!(
        held && !seen.contains(&Relation::Friend),
        "the withdrawn Allied counts in the next field: Concord sees Halo {seen:?}"
    );
}

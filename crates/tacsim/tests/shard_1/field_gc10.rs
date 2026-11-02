//! **Doc 36 cut GC.10 — relations (F-1, F-2)** (Q-36-10, Q-36-11), over the composed world
//! (`driving`'s keyed rig on Ahuna Flank), the arena's egress pumped by hand as `field_host.rs`
//! pumps it.
//!
//! - **Friends by organization** (Q-36-10): two parties whose leaders share an organization are
//!   Friends, and a raider's organization (GC.11's Skulk Syndicate, its leader the principal)
//!   befriends a player who openly stands in it. A party of one is the degenerate roster (F-5).
//!   Friends rank below a mark and below fire-derived hostility.
//! - **Standing at demotion, and the void** (Q-36-11): fire one party observed moves the standing
//!   of the shooter's principals with the victim's factions by a bounded amount; fire on a friend
//!   is offered to the shooter party's leader to void, and the void is recorded beside the change.
//!
//! **What is a fixture.** The armed hulls are made and seated by staff hand (CF.2's fixture), the
//! memberships are written by `@faction/add`, and the raider is seeded by `@field/raiders`, the
//! content's own path. Everything after that is the production field.

use mush_core::{GOD, ObjId};
use tacsim::arena::PartyId;
use tacsim::arena::ledger::LedgerEvent;
use tacsim::arena::party::{Mark, Relation, Relations};
use tacsim::field::{FieldParty, PartyKey};

use crate::driving::{
    AHUNA, Rig, cid, command, land, launched, on_pad, on_pad_as, run, staff_hand, stocked_tuned,
    to_cell, warm,
};
use crate::field_host::{
    armed, egress, frame, seat, second, site_record, task, tune, until, weapon_of,
};

/// The raider the content ships, and its organization (GC.11).
const SKULKS: &str = "tacsim.raider.ahuna_skulks";
/// The Skulks' organization.
const SYNDICATE: &str = "Skulk Syndicate";

/// The parties on Ahuna Flank's ground now, and the Friends among them.
fn friends(rig: &mut Rig) -> (Vec<FieldParty>, Vec<(PartyId, PartyId)>) {
    staff_hand(rig, |ctx| {
        tacsim::field::friends_on_ground(&*ctx, &cid(AHUNA))
    })
}

/// The letter of the party `key` names.
fn letter(parties: &[FieldParty], key: impl Fn(&PartyKey) -> bool) -> PartyId {
    parties
        .iter()
        .find(|party| key(&party.key))
        .map(|party| party.party)
        .unwrap_or_else(|| panic!("no such party in {parties:?}"))
}

/// Whether `a` and `b` are a friendship.
fn paired(pairs: &[(PartyId, PartyId)], a: PartyId, b: PartyId) -> bool {
    pairs.contains(&(a.min(b), a.max(b)))
}

/// A faction's id by name.
fn faction(rig: &Rig, name: &str) -> mush_faction::FactionId {
    rig.tw
        .world()
        .module_state::<mush_faction::FactionState>()
        .and_then(|state| state.faction_by_name(name))
        .map(|faction| faction.id)
        .unwrap_or_else(|| panic!("the faction {name}"))
}

/// `who`'s public score with `faction`.
fn score(rig: &Rig, faction: mush_faction::FactionId, who: ObjId) -> i16 {
    mush_faction::public_standing(rig.tw.world(), faction, who).map_or(0, |view| view.score)
}

/// **Seed the content's raider**, which registers its organization with its leader as the
/// principal; with `launch`, its hulls stand beside the ramp.
fn seed(rig: &mut Rig, launch: bool) {
    let seeded = run(&mut rig.tw, GOD, &format!("@field/raiders {SKULKS}"));
    assert!(seeded.contains("seeded"), "{seeded}");
    if launch {
        let out = run(&mut rig.tw, GOD, &format!("@field/raiders/launch {SKULKS}"));
        assert!(out.contains("launch from"), "{out}");
    }
}

/// `who` joins `faction` openly.
fn join(rig: &mut Rig, faction: &str, who: ObjId) {
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@faction/add {faction}=#{}", who.0),
    );
    assert!(
        rig.tw
            .world()
            .module_state::<mush_faction::FactionState>()
            .is_some_and(|state| state.is_member(who, self::faction(rig, faction))),
        "#{} joins {faction}: {out}",
        who.0
    );
}

/// **GC.10's first gate** (Q-36-10): two rosters in one organization are Friends, a member of a
/// raider's organization is the raider's friend, and nobody else is befriended.
#[test]
fn gc10_two_rosters_in_one_organization_are_friends() {
    let mut rig = stocked_tuned(tune);
    // A multi-party ground: A's and B's armed hulls, the three carriers, the Skulks.
    let (a, _, _) = on_pad(&mut rig, 1);
    let (b, _, _) = on_pad_as(&mut rig, "Other", 2, "Tally Moth", 1);
    // C stays aboard its carrier on the pad: the party C leads is the carrier's.
    let (c, c_row, _) = on_pad_as(&mut rig, "Third", 3, "Lone Moth", 1);
    let carrier_c = ObjId(c_row.hull);
    let hull_a = armed(&mut rig, "Red Hound", (110, 124), 90_000);
    let hull_b = armed(&mut rig, "Grey Hound", (114, 124), 270_000);
    seat(&mut rig, hull_a, a);
    seat(&mut rig, hull_b, b);
    seed(&mut rig, true);
    let raider_leader = rig
        .tw
        .world()
        .module_state::<mush_faction::FactionState>()
        .and_then(|state| state.faction_by_name(SYNDICATE))
        .and_then(|faction| faction.principals.first().copied())
        .expect("the Skulks' organization has its leader as principal");

    // No organization yet: A's and B's hulls are strangers, and so are C and the Skulks.
    let (parties, pairs) = friends(&mut rig);
    let pa = letter(&parties, |key| *key == PartyKey::Unit { unit: hull_a });
    let pb = letter(&parties, |key| *key == PartyKey::Unit { unit: hull_b });
    let pc = letter(&parties, |key| *key == PartyKey::Unit { unit: carrier_c });
    let pr = letter(&parties, |key| {
        *key == PartyKey::Fixture {
            leader: raider_leader,
        }
    });
    assert!(parties.len() >= 4, "a multi-party ground: {parties:?}");
    assert!(!paired(&pairs, pa, pb), "{pairs:?}");
    assert!(!paired(&pairs, pc, pr), "{pairs:?}");

    // A and B join one organization; C joins the Skulks' openly.
    run(&mut rig.tw, GOD, "@faction/create Concord");
    join(&mut rig, "Concord", a);
    join(&mut rig, "Concord", b);
    join(&mut rig, SYNDICATE, c);
    let (parties, pairs) = friends(&mut rig);
    assert_eq!(
        letter(&parties, |key| *key == PartyKey::Unit { unit: hull_a }),
        pa,
        "membership never re-letters a party"
    );
    assert!(paired(&pairs, pa, pb), "one organization: {pairs:?}");
    assert!(
        paired(&pairs, pc, pr),
        "a raider's organization: C {pc:?}, the Skulks {pr:?}: {pairs:?} of {parties:?}"
    );
    for (x, y) in [(pa, pc), (pa, pr), (pb, pc), (pb, pr)] {
        assert!(
            !paired(&pairs, x, y),
            "{x:?} and {y:?} share nothing: {pairs:?}"
        );
    }

    // The order the kernel keeps (Q-36-10): a friend below a mark and below fire. The raider's
    // standing Hostile mark on C still decides the raider's view; C holds the raider a friend
    // until it is fired on.
    let mut books = Relations::new();
    for party in &parties {
        books.admit(party.party);
    }
    for (x, y) in &pairs {
        books.befriend(*x, *y);
    }
    books.mark(pr, pc, Some(Mark::Hostile));
    assert_eq!(books.relation(pa, pb), Relation::Friend);
    assert_eq!(books.relation(pc, pr), Relation::Friend);
    assert_eq!(
        books.relation(pr, pc),
        Relation::Hostile,
        "a mark outranks a friend"
    );
    books.note_fire(pa, pb);
    assert_eq!(
        books.relation(pb, pa),
        Relation::Hostile,
        "fire outranks a friend"
    );
    assert_eq!(
        books.relation(pa, pb),
        Relation::Friend,
        "the shooter's own book is unmoved"
    );
}

/// **GC.10's second gate** (Q-36-11): A fires on B, a fellow member of the Skulks' organization.
/// At demotion A's standing with it falls by the bounded amount, the change names A as its voider,
/// B may not void it, and A's `+field/void` cancels it and is recorded beside it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn gc10_a_leader_voids_friendly_fire_standing() {
    let mut rig = stocked_tuned(|cfg| {
        tune(cfg);
        cfg.field.disengage_s = 5;
        // GC.2's idle bound: no shot for this long demotes whatever contacts stand in reach.
        cfg.field.idle_demote_s = 8;
    });
    // CF.2's ground: A's and B's armed hulls four cells apart, C's prospector beside them, and
    // the Skulks' carrier on the site (seeded, not launched): four kinds of party.
    let (a, _, _) = on_pad(&mut rig, 1);
    let (b, _, _) = on_pad_as(&mut rig, "Other", 2, "Tally Moth", 1);
    let (c, _, c_tvs) = on_pad_as(&mut rig, "Third", 3, "Lone Moth", 1);
    launched(&mut rig, c, c_tvs[0]);
    to_cell(&mut rig, c_tvs[0], (116, 116));
    let hull_a = armed(&mut rig, "Red Hound", (110, 124), 90_000);
    let hull_b = armed(&mut rig, "Grey Hound", (114, 124), 270_000);
    seat(&mut rig, hull_a, a);
    seat(&mut rig, hull_b, b);
    seed(&mut rig, false);
    join(&mut rig, SYNDICATE, a);
    join(&mut rig, SYNDICATE, b);
    let syndicate = faction(&rig, SYNDICATE);
    // The letters the field will give A's and B's hulls: admission letters follow the keys.
    let (parties, pairs) = friends(&mut rig);
    let pa = letter(&parties, |key| *key == PartyKey::Unit { unit: hull_a });
    let pb = letter(&parties, |key| *key == PartyKey::Unit { unit: hull_b });
    assert!(
        paired(&pairs, pa, pb),
        "A and B are friends by the Syndicate: {pairs:?}"
    );
    warm(&mut rig);
    let mut rx = egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();
    for _ in 0..3 {
        let _ = second(&mut rig);
    }

    // A's fire order at B's hull to the east opens the field; the round leaves the barrel.
    let (out, queued) = command(&mut rig, a, "+field/fire E");
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    land(&mut rig, &queued[0]).expect("the promotion lands");
    assert!(
        until(&mut rig, &mut rx, &mut lines, 15, |rig, _| task(rig)
            .is_some())
        .await,
        "the field's task opens"
    );
    let gun = weapon_of(&rig, hull_a);
    let rounds = |rig: &Rig| {
        frame(rig, a).and_then(|frame| {
            frame
                .weapons
                .iter()
                .find(|row| row.id.as_str() == gun)
                .map(|row| row.rounds)
        })
    };
    assert!(
        until(&mut rig, &mut rx, &mut lines, 20, |rig, _| rounds(rig)
            .is_some())
        .await,
        "A's cockpit frame arrives"
    );
    let full = rounds(&rig).expect("A's rounds");
    assert!(
        until(&mut rig, &mut rx, &mut lines, 30, |rig, _| rounds(rig)
            .is_some_and(|left| left < full))
        .await,
        "A's opening shot is fired"
    );
    let arrived = |lines: &[LedgerEvent]| {
        lines.iter().any(|line| {
            matches!(
                line.kind,
                tacsim::arena::ledger::EventKind::Impact { unit, target, .. }
                    if unit == hull_a && target == hull_b
            )
        })
    };
    // Both leaders stand down with a Friendly mark (the hostility friendly fire produced), and
    // with no further shot the field demotes on its idle bound.
    let (out, _) = command(
        &mut rig,
        a,
        &format!("+field/mark {}=friendly", pb.letter()),
    );
    assert!(!out.contains("Usage"), "{out}");
    let (out, _) = command(
        &mut rig,
        b,
        &format!("+field/mark {}=friendly", pa.letter()),
    );
    assert!(!out.contains("Usage"), "{out}");
    assert_eq!(
        score(&rig, syndicate, a),
        0,
        "nothing moves before demotion"
    );
    assert!(
        until(&mut rig, &mut rx, &mut lines, 90, |rig, _| task(rig)
            .is_none()
            && site_record(rig)
                .is_some_and(|record| record.open_field().is_none()))
        .await,
        "the field demotes"
    );
    // The black box's chunks close on fill or at the end, so the round's line is read here.
    assert!(arrived(&lines), "the round reached B's hull");

    // The change: A's standing with the Syndicate, by a point a round and never past the cap;
    // the receipt names A as the one who may void it. Nobody else's standing moved.
    let ordinal = 1;
    let prefix = tacsim::field::standing::cause_prefix(&cid(AHUNA), ordinal);
    let causes = mush_faction::standing_causes(rig.tw.world(), &prefix);
    assert_eq!(causes.len(), 1, "one change, A's: {causes:?}");
    let receipt = mush_faction::standing_receipt(rig.tw.world(), &causes[0]).expect("the receipt");
    assert_eq!(receipt.subject, a);
    assert_eq!(receipt.voidable_by, Some(a), "friendly fire: A may void it");
    assert_eq!(receipt.voided_by, None);
    assert_eq!(receipt.effects.len(), 1, "{:?}", receipt.effects);
    assert_eq!(receipt.effects[0].faction, syndicate);
    let delta = receipt.effects[0].delta;
    assert!(
        (tacsim::field::standing::STANDING_FIELD_CAP..=-1).contains(&delta),
        "bounded: {delta}"
    );
    assert_eq!(score(&rig, syndicate, a), delta);
    assert_eq!(score(&rig, syndicate, b), 0, "B fired nothing");

    // B may not void A's change; A voids it; a second void finds nothing.
    let refused = run(&mut rig.tw, b, "+field/void");
    assert!(
        refused.contains(tacsim::field::standing::NOTHING_TO_VOID),
        "{refused}"
    );
    assert_eq!(score(&rig, syndicate, a), delta);
    let voided = run(&mut rig.tw, a, "+field/void");
    assert!(voided.contains("Voided as friendly fire"), "{voided}");
    assert_eq!(score(&rig, syndicate, a), 0, "the void cancels the change");
    let receipt = mush_faction::standing_receipt(rig.tw.world(), &causes[0]).expect("the receipt");
    assert_eq!(
        receipt.voided_by,
        Some(a),
        "the void is recorded beside the change"
    );
    let again = run(&mut rig.tw, a, "+field/void");
    assert!(
        again.contains(tacsim::field::standing::NOTHING_TO_VOID),
        "{again}"
    );
    let history = run(&mut rig.tw, a, &format!("+standing {SYNDICATE}"));
    assert!(history.contains("(voided)"), "{history}");
    let _ = c;
}

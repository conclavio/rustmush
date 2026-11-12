//! **Doc 45 cut AP.2 — the void waits for an absent leader** (Q-45-20), over the composed world
//! (`driving`'s keyed rig), the arena's egress pumped by hand as `field_fr2.rs` pumps it, at both
//! site sizes.
//!
//! Three parties on one site: **Pilot** and **Other**, fellow members of one faction and so
//! standing Friends, and **Third**, who is neither's friend and fires nothing. Pilot's fire
//! order lands a round on Other, both stand down, and **Pilot logs off before the field
//! settles**. At the demotion Pilot's standing falls exactly as it does for a connected leader
//! (`field_fr2.rs`), and the change names Pilot as its voider, but its window ends 28 IC days on,
//! not ten IC minutes. Then, well past ten IC minutes:
//!
//! - **taken** (256 and 512): Pilot logs in, is told the offer waits, and `+field/void` cancels
//!   the change;
//! - **let lapse** (512): Pilot logs in, lets ten IC minutes pass and is refused; logging in
//!   again inside the 28 IC days, Pilot is offered it again and takes it;
//! - **the 28 IC days** (256): the clock is carried past them; Pilot logs in, is offered nothing
//!   and is refused, and the change stands.
//!
//! A connected leader's ten IC minutes and next-command close are `field_fr2.rs`'s, unchanged.
//!
//! **What is a fixture.** As FR.2's: the armed hulls are made and seated by a staff hand, the
//! faction and its memberships by `@faction/create` and `@faction/add`. IC time is carried
//! forward by the clock (`to_ic`), never waited out. Everything else is the production field.

use mush_core::{GOD, ObjId};
use mush_testkit::TestWorld;
use tacsim::arena::PartyId;
use tacsim::arena::ledger::{EventKind, LedgerEvent};
use tacsim::field::standing::{
    NOTHING_TO_VOID, STANDING_FIELD_CAP, VOID_ABSENT_IC_S, VOID_WAITING, VOID_WINDOW_IC_S,
    cause_prefix,
};
use tacsim::field::{FieldParty, PartyKey};

use crate::driving_rig::{
    AHUNA, Rig, cid, command, ic_now, land, on_pad_as_at, run, said, staff_hand, stocked_tuned,
    to_ic, warm,
};
use crate::field_host_rig::{
    armed_on, egress, frame, seat, second, site_record_at, task, tune, until, weapon_of,
};

/// The faction Pilot and Other both stand in.
const FACTION: &str = "Undertow";

/// **One size the gate runs at** (doc 43 rule 5): the site, its `+ship/undock` word, and three
/// hulls on one row: Third's, Pilot's four cells east of it facing east, Other's four cells
/// east of that.
struct At {
    entry: &'static str,
    venue: &'static str,
    c: (i64, i64),
    a: (i64, i64),
    b: (i64, i64),
}

/// Ahuna Flank at 256 a side: GC.10's row.
const AHUNA_FLANK: At = At {
    entry: AHUNA,
    venue: "ahuna",
    c: (106, 124),
    a: (110, 124),
    b: (114, 124),
};

/// Urvara Reach at 512 a side: row 268 across the 256 line (`field_story`'s fight).
const URVARA_REACH: At = At {
    entry: "urvara_port",
    venue: "urvara",
    c: (246, 268),
    a: (250, 268),
    b: (254, 268),
};

/// What Pilot does once the field has demoted without them.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Then {
    /// Logs in and voids.
    Taken,
    /// Logs in, lets ten IC minutes pass, is refused; logs in again and voids.
    LetLapse,
    /// Logs in 28 IC days after the demotion.
    Capped,
}

/// Drop every connection of `who` and end their session (`excursions.rs`'s helper).
fn disconnect(tw: &mut TestWorld, who: ObjId) {
    let mut out = mush_core::Outbox::new();
    let mut fx = mush_core::Effects::new();
    let conns: Vec<u64> = (1..=64)
        .filter(|conn| tw.engine().actor_of(*conn) == Some(who))
        .collect();
    assert!(!conns.is_empty(), "{who} holds a connection");
    for conn in conns {
        tw.engine_mut().step(
            mush_core::Inbound::ConnDown {
                conn,
                actor: who,
                reason: mush_core::DisconnectKind::Lost,
            },
            &mut out,
            &mut fx,
        );
    }
    tw.engine_mut().step(
        mush_core::Inbound::SessionGone { actor: who },
        &mut out,
        &mut fx,
    );
}

/// The parties on the site's ground now, and the Friends among them.
fn friends(rig: &mut Rig, entry: &str) -> (Vec<FieldParty>, Vec<(PartyId, PartyId)>) {
    staff_hand(rig, |ctx| {
        tacsim::field::friends_on_ground(&*ctx, &cid(entry))
    })
}

/// The letter of the party of one that `unit` is.
fn letter(parties: &[FieldParty], unit: ObjId) -> PartyId {
    parties
        .iter()
        .find(|party| party.key == PartyKey::Unit { unit })
        .map(|party| party.party)
        .unwrap_or_else(|| panic!("no party of #{} in {parties:?}", unit.0))
}

/// The faction's id.
fn faction(rig: &Rig) -> mush_faction::FactionId {
    rig.tw
        .world()
        .module_state::<mush_faction::FactionState>()
        .and_then(|state| state.faction_by_name(FACTION))
        .map(|faction| faction.id)
        .unwrap_or_else(|| panic!("the faction {FACTION}"))
}

/// `who`'s public score with the faction.
fn score(rig: &Rig, who: ObjId) -> i16 {
    mush_faction::public_standing(rig.tw.world(), faction(rig), who).map_or(0, |view| view.score)
}

/// The offer's line, for `n` waiting changes.
fn offer(n: usize) -> String {
    VOID_WAITING.replace("{n}", &n.to_string())
}

/// **The gate at `at`**, then `then` (this file's head).
#[allow(clippy::too_many_lines)] // one field, in order
async fn waiting_gate(at: &At, then: Then) {
    let mut rig = stocked_tuned(|cfg| {
        tune(cfg);
        cfg.field.disengage_s = 5;
        cfg.field.idle_demote_s = 8;
        // The black box's chunks close when full: summaries every pulse fill one in a few arena
        // seconds, so the round's chunk reaches the host while the field runs.
        cfg.arena.ledger.summary_ms = 100;
    });
    let site = (at.venue, at.entry);
    let (a, _, _) = on_pad_as_at(&mut rig, ("Pilot", 1, "Salt Moth"), site, 1);
    let (b, _, _) = on_pad_as_at(&mut rig, ("Other", 2, "Tally Moth"), site, 1);
    let (c, _, _) = on_pad_as_at(&mut rig, ("Third", 3, "Lone Moth"), site, 1);
    let hull_a = armed_on(&mut rig, at.entry, "Red Hound", at.a, 90_000);
    let hull_b = armed_on(&mut rig, at.entry, "Grey Hound", at.b, 270_000);
    let hull_c = armed_on(&mut rig, at.entry, "Dun Hound", at.c, 90_000);
    seat(&mut rig, hull_a, a);
    seat(&mut rig, hull_b, b);
    seat(&mut rig, hull_c, c);
    let made = run(&mut rig.tw, GOD, &format!("@faction/create {FACTION}"));
    for who in [a, b] {
        let out = run(
            &mut rig.tw,
            GOD,
            &format!("@faction/add {FACTION}=#{}", who.0),
        );
        assert!(
            rig.tw
                .world()
                .module_state::<mush_faction::FactionState>()
                .is_some_and(|state| state.is_member(who, faction(&rig))),
            "#{} joins {FACTION}: {made} / {out}",
            who.0
        );
    }
    let (parties, pairs) = friends(&mut rig, at.entry);
    let (pa, pb, pc) = (
        letter(&parties, hull_a),
        letter(&parties, hull_b),
        letter(&parties, hull_c),
    );
    assert!(
        pairs.contains(&(pa.min(pb), pa.max(pb))),
        "Pilot and Other are friends by the faction: {pairs:?}"
    );
    assert!(
        !pairs.contains(&(pa.min(pc), pa.max(pc))) && !pairs.contains(&(pb.min(pc), pb.max(pc))),
        "Third is nobody's friend: {pairs:?}"
    );
    warm(&mut rig);
    let mut rx = egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();
    for _ in 0..3 {
        let _ = second(&mut rig);
    }

    // Pilot's fire order at Other's hull to the east opens the field, and the round reaches it.
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
    assert!(
        until(&mut rig, &mut rx, &mut lines, 20, |rig, _| frame(rig, a)
            .is_some_and(|frame| frame
                .weapons
                .iter()
                .any(|row| row.id.as_str() == gun)))
        .await,
        "Pilot's cockpit frame arrives"
    );
    assert!(
        until(&mut rig, &mut rx, &mut lines, 40, |_, lines| lines
            .iter()
            .any(
                |line| matches!(line.kind, EventKind::Impact { unit, target, .. }
                if unit == hull_a && target == hull_b)
            ))
        .await,
        "the round reaches Other's hull, and the host has counted it"
    );

    // Both stand down with a Friendly mark. Then Pilot logs off, before the field settles.
    for (who, other) in [(a, pb), (b, pa)] {
        let (out, _) = command(
            &mut rig,
            who,
            &format!("+field/mark {}=friendly", other.letter()),
        );
        assert!(!out.contains("Usage"), "{out}");
    }
    disconnect(&mut rig.tw, a);
    let entry = at.entry;
    assert!(
        until(&mut rig, &mut rx, &mut lines, 90, |rig, _| task(rig)
            .is_none()
            && site_record_at(rig, entry)
                .is_some_and(|record| record.open_field().is_none()))
        .await,
        "the field demotes without Pilot"
    );

    // Standing moved exactly as for a connected leader: one change, Pilot's, bounded, naming
    // Pilot as its voider. Only its window differs: 28 IC days from the demotion.
    let prefix = cause_prefix(&cid(entry), 1);
    let causes = mush_faction::standing_causes(rig.tw.world(), &prefix);
    assert_eq!(causes.len(), 1, "one change, Pilot's: {causes:?}");
    let receipt = mush_faction::standing_receipt(rig.tw.world(), &causes[0]).expect("the receipt");
    assert_eq!(receipt.subject, a);
    assert_eq!(
        receipt.voidable_by,
        Some(a),
        "friendly fire: Pilot may void it"
    );
    assert_eq!(receipt.voided_by, None);
    assert_eq!(receipt.effects.len(), 1, "{:?}", receipt.effects);
    let delta = receipt.effects[0].delta;
    assert!(
        (STANDING_FIELD_CAP..=-1).contains(&delta),
        "bounded: {delta}"
    );
    assert_eq!(score(&rig, a), delta);
    assert_eq!(score(&rig, b), 0, "Other fired nothing");
    let window = receipt.window.expect("the window rides the grant");
    let left = window.until_ic - ic_now(&rig.tw);
    assert!(
        left > VOID_ABSENT_IC_S - 3_600 && left <= VOID_ABSENT_IC_S,
        "the offer waits 28 IC days from the demotion, not ten IC minutes: {left} IC s left"
    );

    // Well past ten IC minutes, with Pilot still away: the offer has not lapsed, and it is
    // nobody else's.
    let later = ic_now(&rig.tw) + 3 * VOID_WINDOW_IC_S;
    let _ = to_ic(&mut rig, later);
    let refused = run(&mut rig.tw, b, "+field/void");
    assert!(refused.contains(NOTHING_TO_VOID), "{refused}");
    assert_eq!(score(&rig, a), delta);

    let voided_by = |rig: &Rig| {
        mush_faction::standing_receipt(rig.tw.world(), &causes[0])
            .expect("the receipt")
            .voided_by
    };
    match then {
        Then::Taken => {
            let events = rig.tw.connect(a);
            let heard = said(&rig.tw, &events, a);
            assert!(heard.contains(&offer(1)), "offered on login: {heard}");
            let voided = run(&mut rig.tw, a, "+field/void");
            assert!(voided.contains("Voided as friendly fire"), "{voided}");
            assert_eq!(score(&rig, a), 0, "the void cancels the change");
            assert_eq!(voided_by(&rig), Some(a), "recorded beside the change");
            let again = run(&mut rig.tw, a, "+field/void");
            assert!(again.contains(NOTHING_TO_VOID), "{again}");
        }
        Then::LetLapse => {
            let events = rig.tw.connect(a);
            let heard = said(&rig.tw, &events, a);
            assert!(heard.contains(&offer(1)), "offered on login: {heard}");
            // Ten IC minutes from the return, and no command of Pilot's: it has lapsed.
            let lapsed = ic_now(&rig.tw) + VOID_WINDOW_IC_S + 60;
            let _ = to_ic(&mut rig, lapsed);
            let late = run(&mut rig.tw, a, "+field/void");
            assert!(
                late.contains(NOTHING_TO_VOID),
                "ten IC minutes after the return: {late}"
            );
            assert_eq!(score(&rig, a), delta, "the change stands");
            assert_eq!(voided_by(&rig), None);
            // Back again inside the 28 IC days: offered again, and taken.
            disconnect(&mut rig.tw, a);
            let next_day = ic_now(&rig.tw) + 86_400;
            let _ = to_ic(&mut rig, next_day);
            let events = rig.tw.connect(a);
            let heard = said(&rig.tw, &events, a);
            assert!(heard.contains(&offer(1)), "offered again: {heard}");
            let voided = run(&mut rig.tw, a, "+field/void");
            assert!(voided.contains("Voided as friendly fire"), "{voided}");
            assert_eq!(score(&rig, a), 0);
        }
        Then::Capped => {
            let _ = to_ic(&mut rig, window.until_ic + 1);
            let events = rig.tw.connect(a);
            let heard = said(&rig.tw, &events, a);
            assert!(
                !heard.contains("+field/void"),
                "28 IC days on, nothing is offered: {heard}"
            );
            let late = run(&mut rig.tw, a, "+field/void");
            assert!(late.contains(NOTHING_TO_VOID), "gone: {late}");
            assert_eq!(score(&rig, a), delta, "the change stands");
            assert_eq!(voided_by(&rig), None);
        }
    }
}

/// **AP.2 at 256**: a leader offline at the demotion is offered the void on login and takes it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ap2_ahuna_an_absent_leader_is_offered_the_void_on_login_and_takes_it() {
    waiting_gate(&AHUNA_FLANK, Then::Taken).await;
}

/// **AP.2 at 256**: 28 IC days after the demotion the offer is gone.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ap2_ahuna_the_offer_is_gone_28_ic_days_after_the_demotion() {
    waiting_gate(&AHUNA_FLANK, Then::Capped).await;
}

/// **AP.2 at 512**: a leader offline at the demotion is offered the void on login and takes it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ap2_urvara_an_absent_leader_is_offered_the_void_on_login_and_takes_it() {
    waiting_gate(&URVARA_REACH, Then::Taken).await;
}

/// **AP.2 at 512**: ten IC minutes after the return the offer lapses, and a later return inside
/// the 28 IC days offers it again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ap2_urvara_the_offer_runs_ten_ic_minutes_from_each_return() {
    waiting_gate(&URVARA_REACH, Then::LetLapse).await;
}

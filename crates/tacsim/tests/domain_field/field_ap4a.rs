//! **Doc 45 unit AP.4a — a leader sets a roster member's posture** (Q-45-26, the owner's ruling
//! of 2026-11-12, on Q-45-19), over the composed world with a detachment's roster flown into a
//! field (`roster_rig::fly`, AP.8's scaffold).
//!
//! 1. [`ap4a_a_leader_orders_a_members_rig_to_guard_and_its_owner_can_set_it_back`]: a field of
//!    three parties. The roster is two players: the captain, who leads, in rig 1, and a second
//!    player in rig 2, an armed rig lent to them. A player who is not the leader is refused the
//!    order. With the roster's comms jammed, the captain's `ORDER ... 2 POSTURE GUARD` does not
//!    arrive and the rig holds what it held. The member's link drops; with the jam lifted the
//!    same order arrives inside their five minutes, the rig's record reads `GUARD`, and the
//!    order is answered WILCO for the rig. Five minutes after the loss the rig is its mind's
//!    and is flown as doc 45 unit AP.6 has a guard: it stays where it stood and returns the
//!    fire of a unit that fires on it. An order to a unit whose player has gone is refused by
//!    the roster's own authority rule. The player reads who set the posture and to what at
//!    login, is seated again, sets it back to `RETURN` from their own seat, and their word
//!    stands.
//! 2. [`ap4a_a_hired_crews_slot_takes_no_posture_order`]: the same order to a slot a hired
//!    crew flies is refused, and nothing is issued.
//!
//! What the stories stand in for: the second player comes aboard the captain's carrier by a
//! staff hand (`roster_rig::Billet::Member`), and the roster's rigs are stood on their cells by
//! one, as every field story places its units.

use mush_core::ObjId;
use mush_tactics::detachment::LeaderKind;
use tacsim::detachment::sortie::FieldObjective;
use tacsim::field::absent::{FIELD_LINKDEAD_GRACE_WALL_S, away_of, minded};
use tacsim::posture::Posture;
use tacsim::posture::order::{
    POSTURE_ORDER_ANSWERED, POSTURE_ORDER_CREW, POSTURE_ORDER_USAGE, POSTURE_ORDERED, POSTURE_WORDS,
};

use crate::driving_rig::{Rig, run};
use crate::field_ap8::{Field, connect, end_session, fired, fired_at, lose_link, opened};
use crate::field_host_rig::frame;
use crate::grounds_rig::AHUNA_FLANK;
use crate::roster_rig::{Billet, Plan, commit, delivered_to, fence, orders_of, reply};
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

/// The posture slot `slot`'s delivered order names, when its delivered order is one.
fn ordered(field: &mut Field, slot: u16) -> Option<(Posture, u64)> {
    delivered_to(&mut field.rig, field.flown.operation, slot).and_then(|held| {
        match held.objective {
            Some(FieldObjective::Posture { posture }) if held.delivered => {
                Some((posture, held.sequence))
            }
            _ => None,
        }
    })
}

/// `who` types a roster order under the captain's fence; what they hear.
fn order_as(field: &mut Field, who: ObjId, words: &str) -> String {
    let captain = field.flown.organiser;
    let fenced = fence(&mut field.rig, captain, &field.flown);
    commit(&mut field.rig, who, &format!("ORDER {fenced} {words}"))
}

/// `who` types a roster order under the captain's fence and is refused it: what they hear.
/// Nothing is submitted.
fn refused_as(field: &mut Field, who: ObjId, words: &str) -> String {
    let captain = field.flown.organiser;
    let fenced = fence(&mut field.rig, captain, &field.flown);
    run(&mut field.rig.tw, who, &format!("ORDER {fenced} {words}"))
}

/// **Story 1** (this module's head).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // one story, in order
async fn ap4a_a_leader_orders_a_members_rig_to_guard_and_its_owner_can_set_it_back() {
    let mut field = opened(
        &Plan {
            who: ("Pilot", 1, "Salt Moth"),
            duty: "combat",
            billets: vec![Billet::Leader, Billet::Member("Wren")],
            commander: None,
        },
        true,
    )
    .await;
    let (captain, member) = (field.flown.organiser, field.flown.operator(2));
    let (lead_rig, rig2) = (field.flown.unit(1), field.flown.unit(2));
    let (other, escort) = (field.other, field.escort);
    assert_ne!(captain, member, "two players fly the roster");
    assert_eq!(field.leader(), Some((captain, LeaderKind::Player)));
    assert_eq!(
        (held(&field.rig, lead_rig), held(&field.rig, rig2)),
        (Posture::Return, Posture::Return)
    );

    // (1) A player who is not the leader is refused, whoever's unit they name.
    let refused = refused_as(&mut field, member, "1 POSTURE GUARD");
    assert!(
        refused.contains("not this detachment's current leader"),
        "{refused}"
    );
    field.wait(5).await;
    assert!(
        orders_of(&mut field.rig, field.flown.operation)
            .iter()
            .all(|held| !matches!(held.objective, Some(FieldObjective::Posture { .. }))),
        "a player who does not lead issues no order: {refused}"
    );
    assert_eq!(held(&field.rig, lead_rig), Posture::Return, "{refused}");

    // (2) The roster's comms are jammed (interference declared where its two players are,
    // as doc 44 unit MW.14a's story jams a roster): the captain's order does not arrive.
    let staff = staff_member(&mut field.rig);
    let places: std::collections::BTreeSet<ObjId> = [captain, member]
        .into_iter()
        .map(|who| place_of(&field.rig, who))
        .collect();
    let jam = |field: &mut Field, level: u8| {
        for place in &places {
            let said = run(
                &mut field.rig.tw,
                staff,
                &format!("@prop/jam #{}={level}", place.0),
            );
            assert!(said.contains("Interference at"), "{said}");
        }
    };
    jam(&mut field, 4);
    let issued = order_as(&mut field, captain, "2 POSTURE GUARD");
    // Forty-five seconds under the jam. The same order, once the jam lifts, is held below to
    // arrive inside thirty: the jam, and not the transport's own pace, is what holds it here.
    field.wait(45).await;
    assert_eq!(
        ordered(&mut field, 2),
        None,
        "a jammed order is not delivered: {issued}; book {:?}",
        orders_of(&mut field.rig, field.flown.operation)
    );
    assert_eq!(
        held(&field.rig, rig2),
        Posture::Return,
        "and the rig holds what it held"
    );

    // (3) The member's link drops. Inside the five minutes the rig is still theirs, and the
    // roster still commands it. The interference lifts: the same order arrives, is written to
    // the rig's own record and is answered for the rig.
    lose_link(&mut field.rig, member);
    let lost = away_of(field.rig.tw.world(), rig2)
        .expect("the field's clock runs")
        .since_wall_s;
    jam(&mut field, 0);
    let arrived = field
        .until(30, |rig, _| held(rig, rig2) == Posture::Guard)
        .await;
    assert!(
        arrived,
        "the leader's order sets the member's rig to GUARD: book {:?}",
        orders_of(&mut field.rig, field.flown.operation)
    );
    assert!(
        field.rig.tw.world().now() < lost + FIELD_LINKDEAD_GRACE_WALL_S
            && !minded(field.rig.tw.world(), rig2),
        "inside the five minutes: the rig is still its player's"
    );
    let (word, sequence) = ordered(&mut field, 2).expect("the POSTURE order is delivered");
    assert_eq!(word, Posture::Guard);
    // The rig answers for itself: the leader's report reads WILCO against slot 2's order.
    let mut answer = None;
    for _ in 0..90 {
        answer = reply(&mut field.rig, captain, &field.flown, 2);
        if answer
            .as_deref()
            .is_some_and(|status| status.to_ascii_uppercase().contains("WILCO"))
        {
            break;
        }
        field.wait(1).await;
    }
    assert!(
        answer
            .as_deref()
            .is_some_and(|status| status.to_ascii_uppercase().contains("WILCO")),
        "the leader's report shows the rig's own answer to slot 2's order: {answer:?}; {}",
        field.report()
    );
    assert_eq!(
        held(&field.rig, lead_rig),
        Posture::Return,
        "the order is for the slot it names"
    );

    // (4) Five minutes after the loss the rig is its mind's, and is flown as a guard. It
    // stays where it stood, fires on nobody, and returns the fire of the unit that fires on
    // it.
    field.pass_to(lost + FIELD_LINKDEAD_GRACE_WALL_S + 5);
    field.wait(15).await;
    assert!(
        minded(field.rig.tw.world(), rig2),
        "five minutes after the loss the member's rig is its mind's"
    );
    assert_eq!(
        fired(&field.lines, rig2),
        0,
        "a guard fires on nobody that has not fired"
    );
    let at = (AHUNA_FLANK.read.0 + 7, AHUNA_FLANK.read.1);
    let at = (
        u16::try_from(at.0).expect("a cell"),
        u16::try_from(at.1).expect("a cell"),
    );
    let mut answered = false;
    for _ in 0..150 {
        field.fire_on((other, escort), at);
        if field
            .until(1, |_, lines| fired_at(lines, rig2, escort) > 0)
            .await
        {
            answered = true;
            break;
        }
    }
    field.cease((other, escort));
    assert!(
        answered,
        "the guard returns the fire of the unit that fired on it: it fired {} round(s)",
        fired(&field.lines, rig2)
    );
    assert!(
        crate::escort_lease::in_field(&field.rig, rig2) && minded(field.rig.tw.world(), rig2),
        "and it is still in the field, where a rig on RETURN would have made for home"
    );

    // (5) A unit whose player has gone is no longer under the roster's command: the same
    // order is refused by the roster's own authority rule, and changes nothing.
    let gone = refused_as(&mut field, captain, "2 POSTURE HOLD");
    assert!(
        gone.contains("command authority does not cover the target unit"),
        "{gone}"
    );
    assert_eq!(held(&field.rig, rig2), Posture::Guard);

    // (6) The server's own grace has ended the session by now. Its player logs in, reads who
    // set it and to what, and is seated again.
    end_session(&mut field.rig, member);
    let heard = connect(&mut field.rig, member);
    let leader = field
        .rig
        .tw
        .world()
        .get(captain)
        .map(|object| object.name.to_string())
        .expect("the captain");
    let told = POSTURE_ORDERED
        .replace("{leader}", &leader)
        .replace("{unit}", &rig2.0.to_string())
        .replace("{posture}", "GUARD");
    assert!(
        heard.contains(&told),
        "the owner reads who set it and to what: {heard}"
    );

    // The order is the unit's to answer: its player's own report offers no reply to it, and
    // a NEGATIVE typed against it is refused.
    let own = run(
        &mut field.rig.tw,
        member,
        &format!(
            "+sortie/orders {}:{}",
            field.flown.operation.organizer.0,
            field.flown.selector()
        ),
    );
    assert!(
        own.contains("POSTURE GUARD") && !own.contains("NEGATIVE DT:"),
        "the member reads the order and is offered no reply to it: {own}"
    );
    let fenced = fence(&mut field.rig, captain, &field.flown);
    let (reply_fence, _) = fenced.rsplit_once(':').expect("a fence with a term");
    let vetoed = run(
        &mut field.rig.tw,
        member,
        &format!("NEGATIVE {reply_fence} {sequence}"),
    );
    assert!(vetoed.contains(POSTURE_ORDER_ANSWERED), "{vetoed}");

    // (7) The owner sets it back from their own seat, and their word stands: an order the
    // rig has answered is not applied again.
    let focus = run(
        &mut field.rig.tw,
        member,
        &format!("+focus #{}/pilot", rig2.0),
    );
    assert!(
        field.until(30, |rig, _| frame(rig, member).is_some()).await,
        "the returning player's cockpit frame arrives: {focus}"
    );
    let back = run(&mut field.rig.tw, member, "MIND POSTURE RETURN");
    assert_eq!(held(&field.rig, rig2), Posture::Return, "{back}");
    field.wait(30).await;
    assert_eq!(
        held(&field.rig, rig2),
        Posture::Return,
        "the owner's own later set wins over the leader's order"
    );
}

/// **Story 2** (this module's head): a hired crew's slot takes no posture order.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ap4a_a_hired_crews_slot_takes_no_posture_order() {
    let mut field = opened(
        &Plan {
            who: ("Pilot", 1, "Salt Moth"),
            duty: "mining",
            billets: vec![Billet::Leader, Billet::Hired("Ilya Mendes")],
            commander: None,
        },
        false,
    )
    .await;
    let captain = field.flown.organiser;
    let rig2 = field.flown.unit(2);
    let refused = refused_as(&mut field, captain, "2 POSTURE GUARD");
    assert!(refused.contains(POSTURE_ORDER_CREW), "{refused}");
    field.wait(5).await;
    assert!(
        orders_of(&mut field.rig, field.flown.operation)
            .iter()
            .all(|held| !matches!(held.objective, Some(FieldObjective::Posture { .. }))),
        "nothing was issued"
    );
    assert_eq!(held(&field.rig, rig2), Posture::Return);
    // A word that is no posture, and an order to every slot, are refused too.
    for (words, why) in [
        ("1 POSTURE FLEE", POSTURE_WORDS),
        ("ALL POSTURE GUARD", POSTURE_ORDER_USAGE),
        ("1 POSTURE GUARD NOW", POSTURE_ORDER_USAGE),
    ] {
        let refused = refused_as(&mut field, captain, words);
        assert!(refused.contains(why), "{words}: {refused}");
    }
    field.wait(5).await;
    assert!(
        orders_of(&mut field.rig, field.flown.operation)
            .iter()
            .all(|held| !matches!(held.objective, Some(FieldObjective::Posture { .. }))),
        "and none of them was issued"
    );
}

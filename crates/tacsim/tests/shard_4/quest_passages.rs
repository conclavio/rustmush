//! Passage favors recognize a real unfinished journey, never a conversation claim.
use super::onboarding_quests::{giver, pump, visit};
use super::*;
use serde_json::{Value, json};

const PASSAGE: &str = "tacsim.quests.safe_passage/quest.ma_safe_passage";

fn progress(rig: &mut Rig, actor: ObjId) -> Option<Value> {
    let key = serde_json::from_value(json!({"source":PASSAGE,"actor":actor})).unwrap();
    let family = handles(&rig.tw).families.quests;
    typed(&mut rig.tw, &family, &key).map(|row| serde_json::to_value(row).unwrap())
}

fn invitation(rig: &mut Rig, actor: ObjId, ma: ObjId) -> String {
    visit(rig, actor, ma);
    let (offer, queued) = command(rig, actor, &format!("+quest/offer #{}={PASSAGE}", ma.0));
    assert!(
        queued.is_empty(),
        "an invitation does not change a booking: {offer}"
    );
    let token = offer
        .split("+quest/accept ")
        .nth(1)
        .unwrap_or_else(|| panic!("{offer}"))
        .split_whitespace()
        .next()
        .unwrap()
        .trim_end_matches(['.', '`']);
    format!("+quest/accept {token}")
}

fn no_offer(rig: &mut Rig, actor: ObjId, ma: ObjId) {
    visit(rig, actor, ma);
    let (offer, queued) = command(rig, actor, &format!("+quest/offer #{}={PASSAGE}", ma.0));
    assert!(queued.is_empty(), "{offer}");
    assert!(
        !offer.contains("+quest/accept "),
        "an unavailable traveller cannot be promised: {offer}"
    );
    assert!(progress(rig, actor).is_none());
}

fn accept(rig: &mut Rig, actor: ObjId, ma: ObjId) {
    let invitation = invitation(rig, actor, ma);
    let (out, queued) = command(rig, actor, &invitation);
    assert_eq!(queued.len(), 1, "{out}");
    land_all(rig, &queued);
    let row = progress(rig, actor).unwrap();
    assert_eq!(row["definition"]["prerequisite"], "open");
    assert!(
        row.get("passage").is_none(),
        "the favor is not a paid-passage prerequisite"
    );
    assert!(row["checks"].as_array().unwrap().is_empty());
}

fn step(rig: &mut Rig, actor: ObjId, ma: ObjId, switch: &str) {
    let (out, queued) = command(rig, actor, &format!("+quest/{switch} #{}={PASSAGE}", ma.0));
    assert_eq!(queued.len(), 1, "{switch}: {out}");
    land_all(rig, &queued);
}

fn refused(rig: &mut Rig, actor: ObjId, ma: ObjId, switch: &str) {
    let before = progress(rig, actor);
    let (out, queued) = command(rig, actor, &format!("+quest/{switch} #{}={PASSAGE}", ma.0));
    assert!(queued.is_empty(), "unverified passage must refuse: {out}");
    assert_eq!(progress(rig, actor), before);
}

fn effects(rig: &Rig, npc: ObjId, actor: ObjId) -> (i16, i16, usize) {
    let host = rig
        .tw
        .world()
        .module_state::<mush_npc::NpcHostState>()
        .unwrap();
    let relationship = host.relationships.personal(npc, actor);
    let outcomes = host.personal_memory.record_for(npc).map_or(0, |record| {
        record
            .experiences
            .iter()
            .filter(|event| {
                event.kind == mush_npc::ExperienceKind::Outcome && event.speaker == Some(actor)
            })
            .count()
    });
    (relationship.trust, relationship.disposition, outcomes)
}

/// Two ordinary voyages with time ashore should not depend on the leased hull's
/// dormancy grace. Buy out through the real settlement before loading or booking.
fn owned_booking() -> (Rig, ObjId, HullRow, ObjId, ObjId, u64) {
    let mut rig = stocked();
    let (actor, hull) = passengers::charter(&mut rig, "SafePassageCaptain", 861, "Safe Passage");
    let account = tacsim::port::credit::own_account(rig.tw.world(), actor)
        .unwrap()
        .id;
    let treasury = port_map(&rig.tw, "ceres_port").treasury.unwrap();
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@econ/mint {treasury}={} 10000000", account.0),
    );
    assert!(!out.contains("Usage"), "{out}");
    let registry = station_room(&rig.tw, "ceres_port", "room.c05_registry");
    teleport(&mut rig.tw, actor, registry);
    let out = run(&mut rig.tw, actor, "+coop/buyout");
    assert!(
        out.contains("The Registry writes the title transfer"),
        "{out}"
    );
    survey_sweep(&mut rig);
    assert_eq!(
        tacsim::port::coop::helm_of(rig.tw.world(), actor),
        Some(tacsim::port::coop::Helm {
            hull: ObjId(hull.hull),
            standing: tacsim::port::coop::Standing::Title,
        })
    );
    load(&mut rig, &hull, 150, 200, 98_400);
    let nadia = mush_atlas::travel::latched_object(
        rig.tw.world(),
        &mush_atlas::RecordKey::new(cid("tacsim.passengers.ceres"), cid("npc.nadia_okafor_sato")),
    )
    .unwrap();
    let ma = giver(&rig.tw, "quest.ma_connections");
    visit(&mut rig, actor, nadia);
    let (booking, _) = passengers::book(&mut rig, actor, nadia, "350");
    (rig, actor, hull, nadia, ma, booking)
}

fn deliver(rig: &mut Rig, actor: ObjId, hull: &HullRow, booking: u64) {
    board(rig, actor);
    let outbound = file_and_launch(rig, actor, ObjId(hull.hull), "+ship/undock stickney");
    passengers::arrive(rig, &outbound, "stickney_port");
    assert_eq!(passengers::booking(rig, booking)["phase"], "Delivered");
    let out = run(&mut rig.tw, actor, "+ship/leave");
    assert!(out.contains("You cycle the lock out into"), "{out}");
}

fn settle(rig: &mut Rig, booking: u64) {
    for _ in 0..8 {
        if matches!(
            survey_contract(&rig.tw, booking),
            mush_economy::ContractState::Settled { .. }
        ) {
            break;
        }
        survey_sweep(rig);
    }
    assert!(matches!(
        survey_contract(&rig.tw, booking),
        mush_economy::ContractState::Settled { .. }
    ));
    pump(rig);
}

fn return_to_ma(rig: &mut Rig, actor: ObjId, hull: &HullRow, ma: ObjId) {
    let gallery = station_room(&rig.tw, "stickney_port", "room.p01_berth_gallery");
    teleport(&mut rig.tw, actor, gallery);
    let out = run(&mut rig.tw, actor, "+ship/board");
    assert!(out.contains("You cycle the lock into"), "{out}");
    let inbound = file_and_launch(rig, actor, ObjId(hull.hull), "+ship/undock ceres");
    passengers::arrive(rig, &inbound, "ceres_port");
    let out = run(&mut rig.tw, actor, "+ship/leave");
    assert!(out.contains("You cycle the lock out into"), "{out}");
    visit(rig, actor, ma);
}

fn replace_booking(rig: &mut Rig, booking: u64, row: Value) {
    let family = handles(&rig.tw).families.passengers;
    let key = serde_json::from_value(json!(booking)).unwrap();
    let row = serde_json::from_value(row).unwrap();
    let engine = rig.tw.engine_mut();
    let prepared = {
        let mut txn = engine.action_transaction(&family).unwrap();
        txn.replace(&family, &key, row).unwrap();
        txn.prepare().unwrap()
    };
    let (ops, install) = prepared.into_parts();
    engine.install_prepared_action(install).unwrap();
    keep(rig, ops);
}

fn stale_booking_cannot_commit(rig: &mut Rig, actor: ObjId, ma: ObjId, booking: u64, switch: &str) {
    let before = progress(rig, actor);
    let (out, queued) = command(rig, actor, &format!("+quest/{switch} #{}={PASSAGE}", ma.0));
    assert_eq!(queued.len(), 1, "{out}");
    let resolved = resolve_with_guards(rig, &queued[0]).unwrap();
    let original = passengers::booking(rig, booking);
    let mut changed = original.clone();
    changed["revision"] = json!(original["revision"].as_u64().unwrap() + 1);
    replace_booking(rig, booking, changed);
    let query = rig
        .tw
        .engine()
        .legal_action_query(
            mush_core::PrincipalId::Object { id: actor },
            actor,
            mush_core::PerceivedRef::SelfActor,
        )
        .unwrap();
    let offers = rig
        .tw
        .engine()
        .project_legal_actions(&query)
        .expect("a stale optional quest cannot break unrelated action projection");
    assert!(
        offers.projected_for_action(&queued[0].action).is_none(),
        "the stale quest action itself must not be offered"
    );
    assert!(
        !rig.tw
            .engine()
            .revalidate_guards(&resolved.guards)
            .is_ok_and(|result| result.is_current()),
        "a stale display and staged proof cannot hide a changed typed booking"
    );
    assert!(
        rig.tw
            .engine_mut()
            .prepare_registered_resolved_authoritative_action(
                &resolved.action,
                &resolved.resolved,
                CommitToken::from_bytes(resolved.token),
            )
            .is_err(),
        "prepare must reread the exact passenger receipt for {switch}"
    );
    assert!(
        land(rig, &queued[0]).is_err(),
        "fresh projection must also reject the old receipt"
    );
    assert_eq!(progress(rig, actor), before);
    replace_booking(rig, booking, original);
}

#[test]
fn booked_passage_requires_delivery_payment_and_exact_receipt_then_rewards_ma_once() {
    let (mut rig, actor, hull, nadia, ma, booking) = owned_booking();
    let stranger = pilot(&mut rig, "NotNadiasCaptain", 862);
    no_offer(&mut rig, stranger, ma);
    let agreement = passengers::booking(&mut rig, booking);
    accept(&mut rig, actor, ma);
    assert_eq!(
        passengers::booking(&mut rig, booking),
        agreement,
        "the favor cannot rewrite an existing agreement"
    );
    refused(&mut rig, actor, ma, "check");
    passengers::invite(&mut rig, actor, nadia, booking);
    refused(&mut rig, actor, ma, "check");
    let payee = agreement["payee"].as_u64().unwrap();
    let balance = survey_money(&rig.tw, payee).0;
    deliver(&mut rig, actor, &hull, booking);
    assert_eq!(
        survey_contract(&rig.tw, booking),
        mush_economy::ContractState::Active,
        "delivery alone precedes the actual fare settlement"
    );
    // Isolate payment authority at the required acknowledgment room. The ship
    // really arrived at Stickney; restore the captain there before sailing home.
    visit(&mut rig, actor, ma);
    refused(&mut rig, actor, ma, "check");
    settle(&mut rig, booking);
    assert_eq!(survey_money(&rig.tw, payee).0, balance + 35_000);
    let delivered = passengers::booking(&mut rig, booking);
    let nadia_room = rig.tw.world().get(nadia).unwrap().location;
    assert_eq!(delivered["captain"], json!(actor));
    assert!(
        delivered["delivered"]["at_ic"].as_i64().unwrap()
            > progress(&mut rig, actor).unwrap()["accepted"]["at_ic"]
                .as_i64()
                .unwrap()
    );
    return_to_ma(&mut rig, actor, &hull, ma);
    let paid_balance = survey_money(&rig.tw, payee).0;
    visit(&mut rig, stranger, ma);
    no_offer(&mut rig, stranger, ma);
    refused(&mut rig, stranger, ma, "check");
    assert_eq!(effects(&rig, ma, stranger), (0, 0, 0));
    stale_booking_cannot_commit(&mut rig, actor, ma, booking, "check");
    step(&mut rig, actor, ma, "check");
    let checked = progress(&mut rig, actor).unwrap();
    assert_eq!(checked["checks"][0]["proof"]["kind"], "paid_passage");
    rig = reboot(rig, &[actor, stranger]);
    pump(&mut rig);
    assert_eq!(progress(&mut rig, actor), Some(checked));
    assert_eq!(effects(&rig, ma, actor), (0, 0, 0));
    stale_booking_cannot_commit(&mut rig, actor, ma, booking, "finish");
    step(&mut rig, actor, ma, "finish");
    assert_eq!(
        effects(&rig, ma, actor),
        (0, 0, 0),
        "completion precedes social reconciliation"
    );
    rig = reboot(rig, &[actor, stranger]);
    pump(&mut rig);
    assert_eq!(effects(&rig, ma, actor), (10, 5, 1));
    assert_eq!(
        survey_money(&rig.tw, payee).0,
        paid_balance,
        "the quest adds no fare or cash reward"
    );
    assert_eq!(passengers::booking(&mut rig, booking), delivered);
    assert_eq!(
        rig.tw.world().get(nadia).unwrap().location,
        nadia_room,
        "recognizing the journey never respawns or recalls the traveller"
    );
    let finished = progress(&mut rig, actor).unwrap();
    assert!(finished["completed"].is_object());
    refused(&mut rig, actor, ma, "finish");
    pump(&mut rig);
    assert_eq!(effects(&rig, ma, actor), (10, 5, 1));
    assert_eq!(progress(&mut rig, actor), Some(finished));
    no_refused_writes(&rig.tw);
}

#[test]
fn already_aboard_passage_can_be_accepted_before_its_actual_arrival() {
    let (mut rig, actor, hull, nadia, ma, booking) = owned_booking();
    passengers::invite(&mut rig, actor, nadia, booking);
    assert_eq!(passengers::booking(&mut rig, booking)["phase"], "Aboard");
    accept(&mut rig, actor, ma);
    refused(&mut rig, actor, ma, "check");
    deliver(&mut rig, actor, &hull, booking);
    settle(&mut rig, booking);
    return_to_ma(&mut rig, actor, &hull, ma);
    step(&mut rig, actor, ma, "check");
    step(&mut rig, actor, ma, "finish");
    pump(&mut rig);
    assert_eq!(effects(&rig, ma, actor), (10, 5, 1));
    assert!(progress(&mut rig, actor).unwrap()["completed"].is_object());
    no_refused_writes(&rig.tw);
}

#[test]
fn arrival_closes_the_invitation_even_before_its_fare_is_paid() {
    let (mut rig, actor, hull, nadia, ma, booking) = owned_booking();
    let old_invitation = invitation(&mut rig, actor, ma);
    passengers::invite(&mut rig, actor, nadia, booking);
    deliver(&mut rig, actor, &hull, booking);
    assert_eq!(
        survey_contract(&rig.tw, booking),
        mush_economy::ContractState::Active
    );
    let delivered = passengers::booking(&mut rig, booking);
    no_offer(&mut rig, actor, ma);
    let (out, queued) = command(&mut rig, actor, &old_invitation);
    assert!(
        queued.is_empty(),
        "an old invitation cannot resurrect an arrived traveller: {out}"
    );
    assert!(progress(&mut rig, actor).is_none());
    refused(&mut rig, actor, ma, "check");
    settle(&mut rig, booking);
    no_offer(&mut rig, actor, ma);
    refused(&mut rig, actor, ma, "check");
    rig = reboot(rig, &[actor]);
    pump(&mut rig);
    no_offer(&mut rig, actor, ma);
    refused(&mut rig, actor, ma, "check");
    assert_eq!(effects(&rig, ma, actor), (0, 0, 0));
    assert_eq!(passengers::booking(&mut rig, booking), delivered);
    no_refused_writes(&rig.tw);
}

#[test]
fn acceptance_rechecks_typed_availability_after_resolution() {
    let (mut rig, actor, _hull, _nadia, ma, booking) = owned_booking();
    let invitation = invitation(&mut rig, actor, ma);
    let (out, queued) = command(&mut rig, actor, &invitation);
    assert_eq!(queued.len(), 1, "{out}");
    let resolved = resolve_with_guards(&rig, &queued[0]).unwrap();
    let original = passengers::booking(&mut rig, booking);
    // Deliberately change the authoritative row under a previously resolved
    // invitation. No passage service, projection refresh or clock tick runs.
    let mut arrived = original.clone();
    arrived["revision"] = json!(original["revision"].as_u64().unwrap() + 1);
    arrived["phase"] = json!("Delivered");
    arrived["journey"] = json!({
        "route":"ceres_stickney",
        "departed_ic":original["accepted_ic"],
        "arrival_ic":ic_now(&rig.tw),
    });
    arrived["delivered"] = json!({"at_ic":ic_now(&rig.tw),"at_utc":rig.tw.world().now()});
    replace_booking(&mut rig, booking, arrived);
    assert!(
        !rig.tw
            .engine()
            .revalidate_guards(&resolved.guards)
            .is_ok_and(|result| result.is_current())
    );
    assert!(
        rig.tw
            .engine_mut()
            .prepare_registered_resolved_authoritative_action(
                &resolved.action,
                &resolved.resolved,
                CommitToken::from_bytes(resolved.token),
            )
            .is_err(),
        "acceptance must read the actual passenger receipt again"
    );
    assert!(land(&mut rig, &queued[0]).is_err());
    assert!(progress(&mut rig, actor).is_none());
    replace_booking(&mut rig, booking, original);
    accept(&mut rig, actor, ma);
    assert!(progress(&mut rig, actor).is_some());
    no_refused_writes(&rig.tw);
}

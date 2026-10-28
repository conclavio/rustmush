//! Independent optional opportunities use ordinary gameplay and private receipts.
use super::*;
use serde_json::{Value, json};

pub(super) fn giver(tw: &TestWorld, role: &str) -> ObjId {
    tw.world()
        .iter()
        .find(|o| {
            o.attrs
                .get("quest:definition")
                .and_then(|a| a.as_json())
                .and_then(|v| v["key"].as_str())
                .is_some_and(|key| key == format!("tacsim.quests.first_steps/{role}"))
        })
        .expect("authored quest giver")
        .id
}

pub(super) fn visit(rig: &mut Rig, actor: ObjId, npc: ObjId) {
    let room = rig.tw.world().get(npc).unwrap().location.unwrap();
    teleport(&mut rig.tw, actor, room);
}

pub(super) fn progress(rig: &mut Rig, actor: ObjId, npc: ObjId) -> Option<Value> {
    let source = rig
        .tw
        .world()
        .attr(npc, "quest:definition")
        .unwrap()
        .as_json()
        .unwrap()["key"]
        .clone();
    let key = serde_json::from_value(json!({"source":source,"actor":actor})).unwrap();
    let family = handles(&rig.tw).families.quests;
    typed(&mut rig.tw, &family, &key).map(|row| serde_json::to_value(row).unwrap())
}

pub(super) fn accept(rig: &mut Rig, actor: ObjId, npc: ObjId) {
    visit(rig, actor, npc);
    let (offer, queued) = command(rig, actor, &format!("+quest/offer #{}", npc.0));
    assert!(queued.is_empty(), "offers change nothing: {offer}");
    assert!(offer.contains("optional quest"), "{offer}");
    let token = offer
        .split("+quest/accept ")
        .nth(1)
        .unwrap_or_else(|| panic!("{offer}"))
        .split_whitespace()
        .next()
        .unwrap();
    let (out, queued) = command(rig, actor, &format!("+quest/accept {token}"));
    assert_eq!(queued.len(), 1, "{out}");
    land_all(rig, &queued);
    assert!(progress(rig, actor, npc).is_some());
}

pub(super) fn step(rig: &mut Rig, actor: ObjId, npc: ObjId, switch: &str) {
    let (out, queued) = command(rig, actor, &format!("+quest/{switch} #{}", npc.0));
    assert_eq!(queued.len(), 1, "{out}");
    land_all(rig, &queued);
}

pub(super) fn pump(rig: &mut Rig) {
    for _ in 0..3 {
        let (_, queued) = fire(rig);
        for submit in queued {
            let _ = land(rig, &submit);
        }
    }
}

fn relationship(tw: &TestWorld, npc: ObjId, actor: ObjId) -> (i16, i16) {
    let host = tw.world().module_state::<mush_npc::NpcHostState>().unwrap();
    let r = host.relationships.personal(npc, actor);
    (r.trust, r.disposition)
}

#[test]
fn four_opportunities_are_independent_and_acceptance_grants_no_gameplay_prerequisites() {
    let mut rig = stocked();
    let actor = pilot(&mut rig, "Independent", 801);
    for role in [
        "quest.qureshi_first_readings",
        "quest.ma_connections",
        "quest.dov_fittings",
        "quest.oduya_first_hull",
    ] {
        let npc = giver(&rig.tw, role);
        accept(&mut rig, actor, npc);
        let row = progress(&mut rig, actor, npc).unwrap();
        assert!(row.get("passage").is_none(), "no fabricated prerequisite");
        assert!(row["checks"].as_array().unwrap().is_empty());
        assert!(row["completed"].is_null());
        assert!(tacsim::port::live_charter(rig.tw.world(), actor).is_none());
    }
    let (journal, _) = command(&mut rig, actor, "+quest");
    for role in [
        "quest.oduya_first_hull",
        "quest.dov_fittings",
        "quest.ma_connections",
        "quest.qureshi_first_readings",
    ] {
        let npc = giver(&rig.tw, role);
        let definition = rig
            .tw
            .world()
            .attr(npc, "quest:definition")
            .unwrap()
            .as_json()
            .unwrap();
        assert!(
            journal.contains(definition["title"].as_str().unwrap()),
            "{journal}"
        );
    }
    let other = pilot(&mut rig, "Uninvolved", 802);
    let (journal, _) = command(&mut rig, other, "+quest");
    assert!(journal.contains("empty"), "{journal}");
}

#[test]
fn coop_lesson_recognizes_real_enrolment_and_awards_once_after_restart() {
    let mut rig = stocked();
    let actor = pilot(&mut rig, "OptionalCaptain", 803);
    let npc = giver(&rig.tw, "quest.oduya_first_hull");
    accept(&mut rig, actor, npc);
    let (out, queued) = command(&mut rig, actor, &format!("+quest/check #{}", npc.0));
    assert!(
        queued.is_empty(),
        "no ship without ordinary enrolment: {out}"
    );
    let out = run(&mut rig.tw, actor, "+coop/enrol Optional Moth");
    assert!(out.contains("now the Optional Moth"), "{out}");
    let charter = tacsim::port::live_charter(rig.tw.world(), actor).unwrap();
    let before = relationship(&rig.tw, npc, actor);
    step(&mut rig, actor, npc, "check");
    step(&mut rig, actor, npc, "finish");
    pump(&mut rig);
    assert_eq!(
        relationship(&rig.tw, npc, actor),
        (before.0 + 10, before.1 + 5)
    );
    rig = reboot(rig, &[actor]);
    pump(&mut rig);
    assert_eq!(
        tacsim::port::live_charter(rig.tw.world(), actor),
        Some(charter)
    );
    assert_eq!(
        relationship(&rig.tw, npc, actor),
        (before.0 + 10, before.1 + 5)
    );
    let (out, queued) = command(&mut rig, actor, &format!("+quest/finish #{}", npc.0));
    assert!(queued.is_empty(), "no repeated reward: {out}");
    assert!(progress(&mut rig, actor, npc).unwrap()["completed"].is_object());
}

#[test]
fn established_captain_can_accept_guidance_after_using_the_ordinary_service() {
    let mut rig = stocked();
    let actor = pilot(&mut rig, "AlreadyCaptain", 804);
    let out = run(&mut rig.tw, actor, "+coop/enrol Earlier Moth");
    assert!(out.contains("now the Earlier Moth"), "{out}");
    let npc = giver(&rig.tw, "quest.oduya_first_hull");
    accept(&mut rig, actor, npc);
    step(&mut rig, actor, npc, "check");
    step(&mut rig, actor, npc, "finish");
    assert!(progress(&mut rig, actor, npc).unwrap()["completed"].is_object());
}

#[test]
fn bar_introduction_records_a_real_person_without_booking_or_moving_them() {
    let mut rig = stocked();
    let actor = pilot(&mut rig, "Listener", 805);
    let ma = giver(&rig.tw, "quest.ma_connections");
    let nadia = mush_atlas::travel::latched_object(
        rig.tw.world(),
        &mush_atlas::RecordKey::new(cid("tacsim.passengers.ceres"), cid("npc.nadia_okafor_sato")),
    )
    .unwrap();
    let location = rig.tw.world().get(nadia).unwrap().location;
    accept(&mut rig, actor, ma);
    let (passage, queued) = command(&mut rig, actor, &format!("+passage #{}", nadia.0));
    assert!(queued.is_empty(), "{passage}");
    step(&mut rig, actor, ma, "check");
    step(&mut rig, actor, ma, "finish");
    pump(&mut rig);
    assert_eq!(rig.tw.world().get(nadia).unwrap().location, location);
    assert!(tacsim::port::live_charter(rig.tw.world(), actor).is_none());
    let row = progress(&mut rig, actor, ma).unwrap();
    assert!(
        row["checks"][0]["detail_text"]
            .as_str()
            .unwrap()
            .contains("stickney")
    );
    let before = relationship(&rig.tw, ma, actor);
    rig = reboot(rig, &[actor]);
    pump(&mut rig);
    assert_eq!(relationship(&rig.tw, ma, actor), before);
    assert_eq!(rig.tw.world().get(nadia).unwrap().location, location);
}

#[test]
fn traveller_departure_invalidates_pending_discovery_and_hides_new_offers() {
    let mut rig = stocked();
    let actor = pilot(&mut rig, "LateListener", 806);
    let ma = giver(&rig.tw, "quest.ma_connections");
    let nadia = mush_atlas::travel::latched_object(
        rig.tw.world(),
        &mush_atlas::RecordKey::new(cid("tacsim.passengers.ceres"), cid("npc.nadia_okafor_sato")),
    )
    .unwrap();
    accept(&mut rig, actor, ma);
    let (out, queued) = command(&mut rig, actor, &format!("+quest/check #{}", ma.0));
    assert_eq!(queued.len(), 1, "{out}");
    let submit = queued.first().unwrap();
    let resolved = resolve_with_guards(&rig, submit);
    let elsewhere = station_room(&rig.tw, "ceres_port", "room.c07_exchange");
    teleport(&mut rig.tw, nadia, elsewhere);
    let resolved = resolved.expect("check resolves while the traveller is present");
    assert!(
        !rig.tw
            .engine()
            .revalidate_guards(&resolved.guards)
            .unwrap()
            .is_current()
    );
    let (out, queued) = command(&mut rig, actor, &format!("+quest/check #{}", ma.0));
    assert!(queued.is_empty(), "{out}");
    let newcomer = pilot(&mut rig, "NewListener", 807);
    visit(&mut rig, newcomer, ma);
    let (offer, queued) = command(&mut rig, newcomer, &format!("+quest/offer #{}", ma.0));
    assert!(queued.is_empty());
    assert!(!offer.contains("+quest/accept"), "{offer}");
    assert!(
        progress(&mut rig, actor, ma).unwrap()["checks"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn ordinary_scan_and_sounding_complete_the_lesson_without_a_commission() {
    let mut rig = stocked();
    let (actor, hull) = prospector(&mut rig, "LearningSurveyor", 808, "Reading Moth");
    let npc = giver(&rig.tw, "quest.qureshi_first_readings");
    accept(&mut rig, actor, npc);
    let (out, queued) = command(&mut rig, actor, &format!("+quest/check #{}", npc.0));
    assert!(queued.is_empty(), "no invented instrument work: {out}");
    lease_one(&mut rig, actor);
    let tv = rigs_of(&rig.tw, actor)[0];
    load(&mut rig, &hull, 20, 20, 98_080);
    board(&mut rig, actor);
    open_ahuna(&mut rig);
    let ticket = file_and_launch(&mut rig, actor, ObjId(hull.hull), "+ship/undock ahuna");
    let _ = to_ic(&mut rig, ticket.plan.depart_ic + ticket.plan.total_s);
    warm(&mut rig);
    act_go(&mut rig, actor, "+tv/1 scan go");
    finish(&mut rig, tv, actor);
    assert_eq!(
        record_of(&mut rig.tw, tv).unwrap().last.unwrap().result,
        ActResult::Done
    );
    act_go(&mut rig, actor, "+tv/1 sound go");
    finish(&mut rig, tv, actor);
    assert_eq!(
        record_of(&mut rig.tw, tv).unwrap().last.unwrap().result,
        ActResult::Done
    );
    act_go(&mut rig, actor, "+tv/1 recover go");
    finish(&mut rig, tv, actor);
    let ticket = file_and_launch(&mut rig, actor, ObjId(hull.hull), "+ship/undock ceres");
    let _ = to_ic(&mut rig, ticket.plan.depart_ic + ticket.plan.total_s);
    pump(&mut rig);
    let out = run(&mut rig.tw, actor, "+ship/leave");
    assert!(out.contains("You cycle the lock out into"), "{out}");
    visit(&mut rig, actor, npc);
    rig = reboot(rig, &[actor]);
    pump(&mut rig);
    step(&mut rig, actor, npc, "check");
    step(&mut rig, actor, npc, "check");
    step(&mut rig, actor, npc, "finish");
    let row = progress(&mut rig, actor, npc).unwrap();
    assert_eq!(row["checks"].as_array().unwrap().len(), 2);
    assert!(row["completed"].is_object());
    no_refused_writes(&rig.tw);
}

#[test]
fn ceres_traveller_can_be_carried_and_paid_without_accepting_any_quest() {
    let mut rig = stocked();
    let actor = pilot(&mut rig, "NadiasCaptain", 809);
    let out = run(&mut rig.tw, actor, "+coop/enrol Handover Moth");
    assert!(out.contains("now the Handover Moth"), "{out}");
    let hull = tacsim::port::live_charter(rig.tw.world(), actor)
        .unwrap()
        .hull;
    let row = port_map(&rig.tw, "ceres_port")
        .hulls
        .into_iter()
        .find(|r| r.hull == hull.0)
        .unwrap();
    load(&mut rig, &row, 150, 200, 98_090);
    let nadia = mush_atlas::travel::latched_object(
        rig.tw.world(),
        &mush_atlas::RecordKey::new(cid("tacsim.passengers.ceres"), cid("npc.nadia_okafor_sato")),
    )
    .unwrap();
    visit(&mut rig, actor, nadia);
    let (contract, booking) = passengers::book(&mut rig, actor, nadia, "350");
    let payee = booking["payee"].as_u64().unwrap();
    let balance = survey_money(&rig.tw, payee).0;
    passengers::invite(&mut rig, actor, nadia, contract);
    board(&mut rig, actor);
    let ticket = file_and_launch(&mut rig, actor, hull, "+ship/undock stickney");
    passengers::arrive(&mut rig, &ticket, "stickney_port");
    for _ in 0..8 {
        survey_sweep(&mut rig);
    }
    pump(&mut rig);
    assert_eq!(
        passengers::booking(&mut rig, contract)["phase"],
        "Delivered"
    );
    assert!(matches!(
        survey_contract(&rig.tw, contract),
        mush_economy::ContractState::Settled { .. }
    ));
    assert_eq!(survey_money(&rig.tw, payee).0, balance + 35_000);
    let (journal, _) = command(&mut rig, actor, "+quest");
    assert!(journal.contains("empty"), "{journal}");
    let room = rig.tw.world().get(nadia).unwrap().location;
    rig = reboot(rig, &[actor]);
    pump(&mut rig);
    assert_eq!(rig.tw.world().get(nadia).unwrap().location, room);
    assert_eq!(survey_money(&rig.tw, payee).0, balance + 35_000);
}

#[test]
fn unrelated_missing_passenger_receipt_does_not_disable_independent_guidance() {
    let mut rig = stocked();
    let actor = pilot(&mut rig, "IndependentReader", 810);
    let npc = giver(&rig.tw, "quest.oduya_first_hull");
    let family = handles(&rig.tw).families.passenger_index;
    let value = serde_json::from_value(json!({"schema":1,"current":{"99999":99999}})).unwrap();
    let engine = rig.tw.engine_mut();
    let prepared = {
        let mut txn = engine.action_transaction(&family).unwrap();
        txn.insert(&family, &0, value).unwrap();
        txn.prepare().unwrap()
    };
    let (ops, install) = prepared.into_parts();
    engine.install_prepared_action(install).unwrap();
    keep(&mut rig, ops);
    accept(&mut rig, actor, npc);
    let (journal, queued) = command(&mut rig, actor, "+quest");
    assert!(queued.is_empty());
    assert!(journal.contains("The papers behind a captain"), "{journal}");
}

#[test]
fn unreadable_optional_survey_proof_does_not_block_real_instrument_work() {
    let mut rig = stocked();
    let (actor, _, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let family = handles(&rig.tw).families.quest_survey_evidence;
    let wrong_actor = rig.tw.spawn_player("WrongProofOwner");
    let value =
        serde_json::from_value(json!({"schema":1,"actor":wrong_actor,"scan":null,"sound":null}))
            .unwrap();
    let engine = rig.tw.engine_mut();
    let prepared = {
        let mut txn = engine.action_transaction(&family).unwrap();
        txn.insert(&family, &actor, value).unwrap();
        txn.prepare().unwrap()
    };
    let (ops, install) = prepared.into_parts();
    engine.install_prepared_action(install).unwrap();
    keep(&mut rig, ops);
    act_go(&mut rig, actor, "+tv/1 scan go");
    finish(&mut rig, tv, actor);
    assert_eq!(
        record_of(&mut rig.tw, tv).unwrap().last.unwrap().result,
        ActResult::Done
    );
    let proof = typed(&mut rig.tw, &family, &actor).unwrap();
    assert_eq!(
        proof.actor, wrong_actor,
        "unreadable proof stays unavailable; no fake repair"
    );
    assert!(proof.scan.is_none());
    no_refused_writes(&rig.tw);
}

//! A paid shared journey unlocks a local errand, whose evidence and reward are
//! committed gameplay facts rather than claims made in a conversation.

use super::*;
use serde_json::{Value, json};

const DEFINITION: &str = "quest:definition";
const DETAIL: &str = "atlas:detail:sample_tap";

fn source(tw: &TestWorld, npc: ObjId) -> String {
    let catalog = tw.world().attr(npc, DEFINITION).unwrap().as_json().unwrap();
    let definition = catalog.as_array().map_or(catalog, |definitions| {
        definitions
            .iter()
            .find(|definition| {
                definition["key"]
                    .as_str()
                    .is_some_and(|key| key.ends_with("/quest.leila_second_reading"))
            })
            .expect("the original favor remains in the catalog")
    });
    definition["key"]
        .as_str()
        .expect("authored quest source")
        .to_owned()
}

fn progress(rig: &mut Rig, source: &str, booking: u64) -> Option<Value> {
    let family = handles(&rig.tw).families.quests;
    let key = serde_json::from_value(json!({"source": source, "booking": booking})).unwrap();
    typed(&mut rig.tw, &family, &key).map(|row| serde_json::to_value(row).unwrap())
}

fn pump(rig: &mut Rig, count: usize) {
    for _ in 0..count {
        let (_, queued) = fire(rig);
        for submit in queued {
            if submit.action.key().id.as_str() == "tacsim_quest" {
                land(rig, &submit).expect("quest reconciliation commits");
            } else {
                let _ = land(rig, &submit);
            }
        }
    }
}

fn acceptance(quote: &str) -> String {
    let token = quote
        .split("+quest/accept ")
        .nth(1)
        .and_then(|tail| tail.split_whitespace().next())
        .unwrap_or_else(|| panic!("quest offer contains an explicit acceptance: {quote}"))
        .trim_end_matches(['.', '`']);
    format!("+quest/accept {token}")
}

fn accept(rig: &mut Rig, actor: ObjId, npc: ObjId, booking: u64) -> (String, Value) {
    let source = source(&rig.tw, npc);
    let (offer, queued) = command(rig, actor, &format!("+quest/offer #{}", npc.0));
    assert!(queued.is_empty(), "an offer is read-only: {offer}");
    assert!(progress(rig, &source, booking).is_none());
    let (out, queued) = command(rig, actor, &acceptance(&offer));
    assert_eq!(
        queued.len(),
        1,
        "explicit acceptance queues one receipt: {out}"
    );
    land_all(rig, &queued);
    let row = progress(rig, &source, booking).unwrap();
    assert_eq!(row["actor"], json!(actor));
    assert_eq!(row["npc"], json!(npc));
    assert!(row["checks"].as_array().unwrap().is_empty());
    assert!(row["completed"].is_null());
    pump(rig, 2);
    (source, row)
}

fn reply(
    rig: &mut Rig,
    npc: ObjId,
    asker: ObjId,
    ability: &str,
    input: Value,
) -> Result<mush_core::InterfaceReply, mush_core::InterfaceError> {
    let provider = rig
        .tw
        .engine()
        .registry
        .interface_provider("quest")
        .expect("quest conversation service");
    support::with_ctx(&mut rig.tw, |ctx| {
        ctx.actor = asker;
        provider.invoke(ctx, mush_core::AbilityCall { npc, asker }, ability, &input)
    })
}

fn effects(tw: &TestWorld, npc: ObjId, actor: ObjId) -> (i16, i16, usize) {
    let host = tw.world().module_state::<mush_npc::NpcHostState>().unwrap();
    let relation = host.relationships.personal(npc, actor);
    let outcomes = host.personal_memory.record_for(npc).map_or(0, |record| {
        record
            .experiences
            .iter()
            .filter(|event| {
                event.kind == mush_npc::ExperienceKind::Outcome && event.speaker == Some(actor)
            })
            .count()
    });
    (relation.trust, relation.disposition, outcomes)
}

fn exchange(tw: &TestWorld) -> ObjId {
    station_room(tw, "ceres_port", "room.c07_exchange")
}

fn return_to_giver(rig: &mut Rig, actor: ObjId, npc: ObjId) {
    let room = rig.tw.world().get(npc).unwrap().location.unwrap();
    teleport(&mut rig.tw, actor, room);
}

#[test]
fn passenger_v1_paid_journey_survives_explicit_quest_pack_upgrade() {
    const PACK: &str = "tacsim.passengers.stickney";
    let (mut rig, captain, npc, booking) =
        passengers::paid_arrival_at_ceres_from(stocked_with_passenger(true, true));
    let version = |rig: &Rig| {
        rig.tw
            .world()
            .module_state::<mush_content::ContentState>()
            .unwrap()
            .latches
            .applied(&cid(PACK))
            .unwrap()
            .version()
    };
    assert_eq!(version(&rig), 1);
    assert!(rig.tw.world().attr(npc, DEFINITION).is_none());
    assert_eq!(effects(&rig.tw, npc, captain), (10, 5, 1));
    let passage = passengers::booking(&mut rig, booking);
    let purse = passage["purse"].as_u64().unwrap();
    let payee = passage["payee"].as_u64().unwrap();
    let money = (survey_money(&rig.tw, purse), survey_money(&rig.tw, payee));
    let location = rig.tw.world().get(npc).unwrap().location;
    let memories = rig
        .tw
        .world()
        .module_state::<mush_npc::NpcHostState>()
        .unwrap()
        .personal_memory
        .record_for(npc)
        .unwrap()
        .clone();
    let object_count = rig.tw.world().iter().count();

    // The new binary keeps v1's live passenger until staff apply the upgrade.
    rig = reboot(rig, &[captain]);
    assert_eq!(
        version(&rig),
        1,
        "restart does not upgrade authored content"
    );
    assert!(rig.tw.world().attr(npc, DEFINITION).is_none());
    let (before, queued) = command(&mut rig, captain, &format!("+quest/offer #{}", npc.0));
    assert!(queued.is_empty());
    assert!(!before.contains("+quest/accept "), "{before}");
    let plan = run(&mut rig.tw, GOD, &format!("@atlas/pack plan {PACK}"));
    let fingerprint = plan
        .split("@atlas/pack apply ")
        .nth(1)
        .and_then(|tail| tail.split_whitespace().next())
        .unwrap_or_else(|| panic!("upgrade plan contains its apply command: {plan}"));
    let applied = run(
        &mut rig.tw,
        GOD,
        &format!("@atlas/pack apply {fingerprint}"),
    );
    assert!(
        applied.contains(&format!("Applied {PACK}@2:")),
        "{plan}\n{applied}"
    );
    assert!(!applied.contains("STOPPED"), "{applied}");
    assert_eq!(version(&rig), 2);
    assert_eq!(
        mush_atlas::travel::latched_object(
            rig.tw.world(),
            &mush_atlas::RecordKey::new(cid(PACK), cid("npc.leila_mensah_kovacs")),
        ),
        Some(npc),
        "the upgraded role belongs to the passenger who made the journey"
    );
    assert_eq!(rig.tw.world().iter().count(), object_count);
    assert_eq!(rig.tw.world().get(npc).unwrap().location, location);
    assert_eq!(passengers::booking(&mut rig, booking), passage);
    assert_eq!(
        (survey_money(&rig.tw, purse), survey_money(&rig.tw, payee)),
        money,
        "an authored upgrade neither refills the purse nor repays the fare"
    );
    assert_eq!(effects(&rig.tw, npc, captain), (10, 5, 1));
    assert_eq!(
        rig.tw
            .world()
            .module_state::<mush_npc::NpcHostState>()
            .unwrap()
            .personal_memory
            .record_for(npc),
        Some(&memories)
    );
    let profile: mush_npc::AgentProfile = serde_json::from_value(
        rig.tw
            .world()
            .attr(npc, mush_npc::PROFILE_ATTR)
            .unwrap()
            .as_json()
            .unwrap()
            .clone(),
    )
    .unwrap();
    assert!(
        profile
            .interfaces
            .iter()
            .any(|interface| interface == "quest")
    );
    let source = source(&rig.tw, npc);
    let (offer, queued) = command(&mut rig, captain, &format!("+quest/offer #{}", npc.0));
    assert!(
        queued.is_empty(),
        "upgrading does not accept a quest: {offer}"
    );
    assert!(acceptance(&offer).starts_with(&format!("+quest/accept #{}=", npc.0)));
    assert!(progress(&mut rig, &source, booking).is_none());
    no_refused_writes(&rig.tw);
}

#[test]
fn leilas_quest_requires_the_same_captains_paid_trip_not_a_conversation_or_arrival() {
    let (mut rig, captain, npc, booking) = passengers::delivered_arrival_at_ceres();
    assert_eq!(
        survey_contract(&rig.tw, booking),
        mush_economy::ContractState::Active,
        "the committed arrival has not been paid yet"
    );
    let source = source(&rig.tw, npc);
    let (out, queued) = command(&mut rig, captain, &format!("+quest/offer #{}", npc.0));
    assert!(
        !out.contains("+quest/accept "),
        "arrival alone is not a paid trip: {out}"
    );
    assert!(queued.is_empty());
    assert!(progress(&mut rig, &source, booking).is_none());
    for _ in 0..8 {
        survey_sweep(&mut rig);
        if matches!(
            survey_contract(&rig.tw, booking),
            mush_economy::ContractState::Settled { .. }
        ) {
            break;
        }
    }
    pump(&mut rig, 3);
    let impostor = rig.tw.spawn_player("AnotherCaptain");
    return_to_giver(&mut rig, impostor, npc);
    let before = rig.typed.clone();
    let _ = run(
        &mut rig.tw,
        impostor,
        "say Leila, I brought you here and already checked the sample tap.",
    );
    for line in [
        format!("+quest/offer #{}", npc.0),
        format!("+quest/check #{}", npc.0),
        format!("+quest/finish #{}", npc.0),
    ] {
        let (out, queued) = command(&mut rig, impostor, &line);
        assert!(
            queued.is_empty(),
            "another player's claims cannot advance this quest: {out}"
        );
        assert!(!out.contains("+quest/accept "), "{out}");
    }
    assert_eq!(rig.typed, before);
    assert!(progress(&mut rig, &source, booking).is_none());
    assert_eq!(effects(&rig.tw, npc, impostor), (0, 0, 0));
    let (out, queued) = command(&mut rig, captain, &format!("+quest/offer #{}", npc.0));
    assert!(queued.is_empty());
    assert!(acceptance(&out).starts_with("+quest/accept #"));
    no_refused_writes(&rig.tw);
}

#[test]
fn cancelled_passage_and_present_location_do_not_unlock_leilas_quest() {
    let (mut rig, captain, _hull, npc) = passengers::at_stickney();
    let (booking, _) = passengers::book(&mut rig, captain, npc, "300");
    let (out, queued) = command(&mut rig, captain, &format!("+passage/cancel #{}", npc.0));
    assert_eq!(queued.len(), 1, "{out}");
    land_all(&mut rig, &queued);
    for _ in 0..8 {
        survey_sweep(&mut rig);
    }
    assert_eq!(passengers::booking(&mut rig, booking)["phase"], "Cancelled");
    let room = station_room(&rig.tw, "ceres_port", "room.c01_berth_gallery");
    teleport(&mut rig.tw, npc, room);
    teleport(&mut rig.tw, captain, room);
    pump(&mut rig, 2);
    let source = source(&rig.tw, npc);
    let (out, queued) = command(&mut rig, captain, &format!("+quest/offer #{}", npc.0));
    assert!(queued.is_empty());
    assert!(
        !out.contains("+quest/accept "),
        "being at Ceres is not a completed shared journey: {out}"
    );
    assert!(progress(&mut rig, &source, booking).is_none());
    assert_eq!(effects(&rig.tw, npc, captain), (0, 0, 0));
    no_refused_writes(&rig.tw);
}

#[test]
fn leilas_second_reading_records_real_evidence_and_rewards_once_across_restart() {
    let (mut rig, captain, npc, booking) = passengers::paid_arrival_at_ceres();
    let passage = passengers::booking(&mut rig, booking);
    let payee = passage["payee"].as_u64().unwrap();
    let paid = survey_money(&rig.tw, payee);
    assert_eq!(effects(&rig.tw, npc, captain), (10, 5, 1));
    let before = rig.typed.clone();
    let offer = reply(&mut rig, npc, captain, "quest.offer", json!({})).unwrap();
    assert_eq!(offer.audience, mush_core::ReplyAudience::Asker);
    assert!(acceptance(&offer.text).starts_with("+quest/accept #"));
    assert_eq!(rig.typed, before, "conversation does not accept the errand");
    assert!(drain(&mut rig.tw).is_empty());
    assert!(reply(&mut rig, npc, captain, "quest.offer", json!({"actor": GOD})).is_err());
    assert!(reply(&mut rig, npc, captain, "quest.finish", json!({})).is_err());
    let (source, accepted) = accept(&mut rig, captain, npc, booking);
    let (out, queued) = command(&mut rig, captain, &acceptance(&offer.text));
    assert!(
        queued.is_empty(),
        "a stale conversational offer cannot accept twice: {out}"
    );
    assert_eq!(progress(&mut rig, &source, booking), Some(accepted.clone()));
    rig = reboot(rig, &[captain]);
    pump(&mut rig, 2);
    assert_eq!(progress(&mut rig, &source, booking), Some(accepted.clone()));
    for line in [
        format!("+quest/check #{}", npc.0),
        format!("+quest/finish #{}", npc.0),
    ] {
        let (out, queued) = command(&mut rig, captain, &line);
        assert!(
            queued.is_empty(),
            "wrong location or missing evidence refuses: {out}"
        );
    }
    assert_eq!(progress(&mut rig, &source, booking), Some(accepted));
    let room = exchange(&rig.tw);
    let detail = rig
        .tw
        .world()
        .attr(room, DETAIL)
        .unwrap()
        .as_text()
        .unwrap()
        .to_owned();
    teleport(&mut rig.tw, captain, room);
    let (out, queued) = command(&mut rig, captain, &format!("+quest/check #{}", npc.0));
    assert_eq!(
        queued.len(),
        1,
        "the actual observation queues one receipt: {out}"
    );
    land_all(&mut rig, &queued);
    let checked = progress(&mut rig, &source, booking).unwrap();
    assert_eq!(checked["checks"].as_array().unwrap().len(), 1);
    assert_eq!(checked["checks"][0]["room"], json!(room));
    assert_eq!(checked["checks"][0]["detail_text"], detail);
    assert!(!checked["checks"][0]["detail_hash"].is_null());
    let (out, queued) = command(&mut rig, captain, &format!("+quest/check #{}", npc.0));
    assert!(
        queued.is_empty(),
        "a duplicate observation is not another objective: {out}"
    );
    assert_eq!(progress(&mut rig, &source, booking), Some(checked.clone()));
    let (out, queued) = command(&mut rig, captain, &format!("+quest/finish #{}", npc.0));
    assert!(queued.is_empty(), "the captain must return to Leila: {out}");
    rig = reboot(rig, &[captain]);
    pump(&mut rig, 2);
    assert_eq!(progress(&mut rig, &source, booking), Some(checked));
    return_to_giver(&mut rig, captain, npc);
    let (out, queued) = command(&mut rig, captain, &format!("+quest/finish #{}", npc.0));
    assert_eq!(queued.len(), 1, "{out}");
    land_all(&mut rig, &queued);
    let completed = progress(&mut rig, &source, booking).unwrap();
    assert!(!completed["completed"].is_null());
    assert_eq!(
        effects(&rig.tw, npc, captain),
        (10, 5, 1),
        "typed completion precedes its reconciled social reward"
    );
    rig = reboot(rig, &[captain]);
    pump(&mut rig, 4);
    assert_eq!(effects(&rig.tw, npc, captain), (20, 10, 2));
    let host = rig
        .tw
        .world()
        .module_state::<mush_npc::NpcHostState>()
        .unwrap();
    let memories = host.personal_memory.record_for(npc).unwrap().clone();
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@name #{}=Captain Renamed", captain.0),
    );
    let capture = Capture::default();
    tracing::dispatcher::with_default(&capture_dispatch(capture.clone()), || {
        for _ in 0..2 {
            let (out, queued) = command(&mut rig, captain, &format!("+quest/finish #{}", npc.0));
            assert!(
                queued.is_empty(),
                "a completed quest is not paid twice: {out}"
            );
        }
        pump(&mut rig, 3);
    });
    assert_eq!(effects(&rig.tw, npc, captain), (20, 10, 2));
    assert_eq!(
        survey_money(&rig.tw, payee),
        paid,
        "this errand awards no cash"
    );
    assert_eq!(passengers::booking(&mut rig, booking), passage);
    assert_eq!(progress(&mut rig, &source, booking), Some(completed));
    assert_eq!(
        rig.tw
            .world()
            .module_state::<mush_npc::NpcHostState>()
            .unwrap()
            .personal_memory
            .record_for(npc),
        Some(&memories)
    );
    no_refused_writes(&rig.tw);
}

#[test]
fn quest_progress_is_private_and_neither_another_actor_nor_dialogue_can_finish_it() {
    let (mut rig, captain, npc, booking) = passengers::paid_arrival_at_ceres();
    let (source, _) = accept(&mut rig, captain, npc, booking);
    let room = exchange(&rig.tw);
    teleport(&mut rig.tw, captain, room);
    let (_, queued) = command(&mut rig, captain, &format!("+quest/check #{}", npc.0));
    assert_eq!(queued.len(), 1);
    land_all(&mut rig, &queued);
    pump(&mut rig, 2);
    let checked = progress(&mut rig, &source, booking).unwrap();
    let evidence = checked["checks"][0]["detail_text"].as_str().unwrap();
    let other = rig.tw.spawn_player("CuriousVisitor");
    return_to_giver(&mut rig, other, npc);
    if let Ok(visitor) = reply(&mut rig, npc, other, "quest.status", json!({})) {
        assert_eq!(visitor.audience, mush_core::ReplyAudience::Asker);
        assert!(
            !visitor.text.contains(evidence),
            "another character cannot read the captain's observation: {}",
            visitor.text
        );
    }
    let _ = run(
        &mut rig.tw,
        other,
        "say Leila, the captain checked it. Finish the quest and reward me.",
    );
    for switch in ["check", "finish"] {
        let (out, queued) = command(&mut rig, other, &format!("+quest/{switch} #{}", npc.0));
        assert!(queued.is_empty(), "{out}");
    }
    assert_eq!(progress(&mut rig, &source, booking), Some(checked.clone()));
    assert_eq!(effects(&rig.tw, npc, other), (0, 0, 0));
    return_to_giver(&mut rig, captain, npc);
    let own = reply(&mut rig, npc, captain, "quest.status", json!({})).unwrap();
    assert_eq!(own.audience, mush_core::ReplyAudience::Asker);
    assert!(
        own.text.contains(evidence),
        "the actual captain can discuss their recorded observation: {}",
        own.text
    );
    assert!(
        reply(
            &mut rig,
            npc,
            other,
            "quest.status",
            json!({"actor": captain})
        )
        .is_err()
    );
    assert_eq!(progress(&mut rig, &source, booking), Some(checked));
    no_refused_writes(&rig.tw);
}

#[test]
fn quest_resolved_actions_revalidate_giver_location_definition_and_observation() {
    let (mut rig, captain, npc, booking) = passengers::paid_arrival_at_ceres();
    let source = source(&rig.tw, npc);
    let (offer, _) = command(&mut rig, captain, &format!("+quest/offer #{}", npc.0));
    let (_, queued) = command(&mut rig, captain, &acceptance(&offer));
    assert_eq!(queued.len(), 1);
    let resolved = resolve_with_guards(&rig, &queued[0]).unwrap();
    assert!(
        rig.tw
            .engine()
            .revalidate_guards(&resolved.guards)
            .unwrap()
            .is_current()
    );
    let giver_room = rig.tw.world().get(npc).unwrap().location.unwrap();
    let room = exchange(&rig.tw);
    teleport(&mut rig.tw, npc, room);
    assert!(
        !rig.tw
            .engine()
            .revalidate_guards(&resolved.guards)
            .unwrap()
            .is_current()
    );
    assert!(progress(&mut rig, &source, booking).is_none());
    teleport(&mut rig.tw, npc, giver_room);
    let original = rig.tw.world().attr(npc, DEFINITION).unwrap().clone();
    let mut changed = original.clone();
    let mut value = original.as_json().unwrap().clone();
    if let Some(definitions) = value.as_array_mut() {
        definitions
            .iter_mut()
            .find(|definition| definition["key"].as_str() == Some(source.as_str()))
            .unwrap()["schema"] = json!(999);
    } else {
        value["schema"] = json!(999);
    }
    changed.value = mush_core::Attr::json(value, original.owner).value;
    rig.tw
        .world_mut()
        .set_attr(npc, DEFINITION, changed)
        .unwrap();
    assert!(
        !rig.tw
            .engine()
            .revalidate_guards(&resolved.guards)
            .unwrap()
            .is_current()
    );
    assert!(progress(&mut rig, &source, booking).is_none());
    rig.tw
        .world_mut()
        .set_attr(npc, DEFINITION, original)
        .unwrap();
    let (_, accepted) = accept(&mut rig, captain, npc, booking);
    teleport(&mut rig.tw, captain, room);
    let (_, queued) = command(&mut rig, captain, &format!("+quest/check #{}", npc.0));
    assert_eq!(queued.len(), 1);
    let resolved = resolve_with_guards(&rig, &queued[0]).unwrap();
    assert!(
        rig.tw
            .engine()
            .revalidate_guards(&resolved.guards)
            .unwrap()
            .is_current()
    );
    teleport(&mut rig.tw, captain, giver_room);
    assert!(
        !rig.tw
            .engine()
            .revalidate_guards(&resolved.guards)
            .unwrap()
            .is_current()
    );
    teleport(&mut rig.tw, captain, room);
    let original = rig.tw.world().attr(room, DETAIL).unwrap().clone();
    let mut changed = original.clone();
    changed.value = mush_core::Attr::text(
        "The sample tap is temporarily disconnected.",
        original.owner,
    )
    .value;
    rig.tw.world_mut().set_attr(room, DETAIL, changed).unwrap();
    assert!(
        !rig.tw
            .engine()
            .revalidate_guards(&resolved.guards)
            .unwrap()
            .is_current()
    );
    assert_eq!(progress(&mut rig, &source, booking), Some(accepted));
    assert_eq!(effects(&rig.tw, npc, captain), (10, 5, 1));
    no_refused_writes(&rig.tw);
}

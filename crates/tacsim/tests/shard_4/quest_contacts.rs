//! General introductions verify the actual people, independently of their services.
use super::onboarding_quests::{giver, pump, visit};
use super::*;
use mush_core::{Flags, ObjKind};
use serde_json::{Value, json};

const CONTACT: &str = "tacsim.quests.port_connections/quest.ma_port_connections";
const TRAVELLER: &str = "tacsim.quests.first_steps/quest.ma_connections";

fn progress(rig: &mut Rig, actor: ObjId, source: &str) -> Option<Value> {
    let key = serde_json::from_value(json!({"source":source,"actor":actor})).unwrap();
    let family = handles(&rig.tw).families.quests;
    typed(&mut rig.tw, &family, &key).map(|row| serde_json::to_value(row).unwrap())
}

fn accept(rig: &mut Rig, actor: ObjId, ma: ObjId) {
    visit(rig, actor, ma);
    let (offer, queued) = command(rig, actor, &format!("+quest/offer #{}={CONTACT}", ma.0));
    assert!(queued.is_empty(), "{offer}");
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
    assert!(progress(rig, actor, CONTACT).is_some());
}

fn step(rig: &mut Rig, actor: ObjId, ma: ObjId, switch: &str) {
    let (out, queued) = command(rig, actor, &format!("+quest/{switch} #{}={CONTACT}", ma.0));
    assert_eq!(queued.len(), 1, "{out}");
    land_all(rig, &queued);
}

fn refused(rig: &mut Rig, actor: ObjId, ma: ObjId) {
    let (out, queued) = command(rig, actor, &format!("+quest/check #{}={CONTACT}", ma.0));
    assert!(queued.is_empty(), "unverified meeting must refuse: {out}");
}

fn effects(rig: &Rig, npc: ObjId, actor: ObjId) -> (i16, i16, usize) {
    let host = rig
        .tw
        .world()
        .module_state::<mush_npc::NpcHostState>()
        .unwrap();
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

#[test]
fn two_real_introductions_need_no_trade_or_charter_and_reward_ma_once() {
    let mut rig = stocked();
    let actor = pilot(&mut rig, "NewFace", 851);
    let stranger = pilot(&mut rig, "OtherFace", 852);
    let ma = giver(&rig.tw, "quest.ma_connections");
    let dov = giver(&rig.tw, "quest.dov_fittings");
    let qureshi = giver(&rig.tw, "quest.qureshi_first_readings");
    let identities: Vec<_> = [ma, dov, qureshi]
        .into_iter()
        .map(|npc| {
            let object = rig.tw.world().get(npc).unwrap();
            (
                npc,
                object.location,
                object.attrs.get(mush_npc::PROFILE_ATTR).unwrap().clone(),
            )
        })
        .collect();
    onboarding_quests::accept(&mut rig, actor, ma);
    accept(&mut rig, actor, ma);
    let (out, queued) = command(&mut rig, actor, &format!("+quest/check #{}", ma.0));
    assert!(
        queued.is_empty(),
        "two active favors require a selector: {out}"
    );
    assert!(out.contains(CONTACT) && out.contains(TRAVELLER), "{out}");
    refused(&mut rig, actor, ma);
    visit(&mut rig, actor, dov);
    step(&mut rig, actor, ma, "check");
    let first = progress(&mut rig, actor, CONTACT).unwrap();
    assert_eq!(first["checks"].as_array().unwrap().len(), 1);
    assert_eq!(first["checks"][0]["proof"]["kind"], "contact");
    assert_eq!(first["checks"][0]["proof"]["npc"], json!(dov));
    assert!(
        progress(&mut rig, actor, TRAVELLER).unwrap()["checks"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    visit(&mut rig, stranger, dov);
    refused(&mut rig, stranger, ma);
    assert!(progress(&mut rig, stranger, CONTACT).is_none());
    rig = reboot(rig, &[actor, stranger]);
    pump(&mut rig);
    assert_eq!(progress(&mut rig, actor, CONTACT), Some(first));
    visit(&mut rig, actor, qureshi);
    step(&mut rig, actor, ma, "check");
    let checked = progress(&mut rig, actor, CONTACT).unwrap();
    assert_eq!(checked["checks"][1]["proof"]["npc"], json!(qureshi));
    let (out, queued) = command(
        &mut rig,
        actor,
        &format!("+quest/finish #{}={CONTACT}", ma.0),
    );
    assert!(queued.is_empty(), "return to the giver: {out}");
    visit(&mut rig, actor, ma);
    step(&mut rig, actor, ma, "finish");
    pump(&mut rig);
    assert_eq!(effects(&rig, ma, actor), (10, 5, 1));
    assert_eq!(effects(&rig, dov, actor), (0, 0, 0));
    assert_eq!(effects(&rig, qureshi, actor), (0, 0, 0));
    assert!(tacsim::port::live_charter(rig.tw.world(), actor).is_none());
    rig = reboot(rig, &[actor, stranger]);
    pump(&mut rig);
    assert_eq!(effects(&rig, ma, actor), (10, 5, 1));
    let (out, queued) = command(
        &mut rig,
        actor,
        &format!("+quest/finish #{}={CONTACT}", ma.0),
    );
    assert!(queued.is_empty(), "completion cannot repeat: {out}");
    assert!(progress(&mut rig, actor, CONTACT).unwrap()["completed"].is_object());
    for (npc, location, profile) in identities {
        let object = rig.tw.world().get(npc).unwrap();
        assert_eq!(object.location, location);
        assert_eq!(object.attrs.get(mush_npc::PROFILE_ATTR), Some(&profile));
    }
    no_refused_writes(&rig.tw);
}

#[test]
fn contact_checks_require_the_bound_live_visible_person_and_invalidate_stale_actions() {
    let mut rig = stocked();
    let actor = pilot(&mut rig, "NeedsAnIntroduction", 853);
    let ma = giver(&rig.tw, "quest.ma_connections");
    let dov = giver(&rig.tw, "quest.dov_fittings");
    let room = rig.tw.world().get(dov).unwrap().location.unwrap();
    let elsewhere = station_room(&rig.tw, "ceres_port", "room.c07_exchange");
    accept(&mut rig, actor, ma);
    teleport(&mut rig.tw, actor, room);
    let (out, queued) = command(
        &mut rig,
        actor,
        &format!("+quest/check #{}={CONTACT}", ma.0),
    );
    assert_eq!(queued.len(), 1, "{out}");
    let resolved = resolve_with_guards(&rig, &queued[0]).unwrap();
    teleport(&mut rig.tw, dov, elsewhere);
    assert!(
        !rig.tw
            .engine()
            .revalidate_guards(&resolved.guards)
            .unwrap()
            .is_current()
    );
    assert!(land(&mut rig, &queued[0]).is_err());
    // Actor names are unique. Release the original spelling through the real
    // rename path before another live NPC takes it; the quest still binds Dov's ID.
    let original_name = rig.tw.world().get(dov).unwrap().name.to_string();
    rig.tw
        .world_mut()
        .set_name(dov, "Off-duty Chandlery Keeper")
        .unwrap();
    let double = rig
        .tw
        .world_mut()
        .create(ObjKind::Npc, &original_name, GOD, Some(room))
        .unwrap();
    let profile = rig
        .tw
        .world()
        .attr(dov, mush_npc::PROFILE_ATTR)
        .unwrap()
        .clone();
    rig.tw
        .world_mut()
        .set_attr(double, mush_npc::PROFILE_ATTR, profile)
        .unwrap();
    refused(&mut rig, actor, ma);
    rig.tw
        .world_mut()
        .set_name(double, "Chandlery Visitor")
        .unwrap();
    rig.tw.world_mut().set_name(dov, &original_name).unwrap();
    teleport(&mut rig.tw, dov, room);
    for flag in [Flags::DARK, Flags::GOING] {
        rig.tw.world_mut().get_mut(dov).unwrap().flags.insert(flag);
        refused(&mut rig, actor, ma);
        rig.tw.world_mut().get_mut(dov).unwrap().flags.remove(flag);
    }
    rig.tw.world_mut().get_mut(dov).unwrap().kind = ObjKind::Thing;
    refused(&mut rig, actor, ma);
    rig.tw.world_mut().get_mut(dov).unwrap().kind = ObjKind::Npc;
    rig.tw
        .world_mut()
        .get_mut(actor)
        .unwrap()
        .flags
        .insert(Flags::DARK);
    refused(&mut rig, actor, ma);
    rig.tw
        .world_mut()
        .get_mut(actor)
        .unwrap()
        .flags
        .remove(Flags::DARK);
    let original = rig
        .tw
        .world()
        .attr(dov, mush_npc::PROFILE_ATTR)
        .unwrap()
        .clone();
    let mut disabled = original.as_json().unwrap().clone();
    disabled["enabled"] = json!(false);
    rig.tw
        .world_mut()
        .set_attr(
            dov,
            mush_npc::PROFILE_ATTR,
            mush_core::Attr::json(disabled, GOD),
        )
        .unwrap();
    refused(&mut rig, actor, ma);
    rig.tw
        .world_mut()
        .set_attr(dov, mush_npc::PROFILE_ATTR, original)
        .unwrap();
    assert!(
        progress(&mut rig, actor, CONTACT).unwrap()["checks"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    step(&mut rig, actor, ma, "check");
    assert_eq!(
        progress(&mut rig, actor, CONTACT).unwrap()["checks"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    no_refused_writes(&rig.tw);
}

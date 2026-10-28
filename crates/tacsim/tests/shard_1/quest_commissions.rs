//! Survey guidance recognizes completed paid work without owning the commission.
use super::onboarding_quests::{giver, pump, visit};
use super::*;
use serde_json::{Value, json};

const QUEST: &str = "tacsim.quests.survey_work/quest.qureshi_signed_work";
const STARTER: &str = "tacsim.quests.first_steps/quest.qureshi_first_readings";
const CELLS: [(i64, i64); 4] = [(16, 208), (48, 208), (16, 240), (48, 240)];

fn progress(rig: &mut Rig, actor: ObjId, source: &str) -> Option<Value> {
    let key = serde_json::from_value(json!({"source":source,"actor":actor})).unwrap();
    let family = handles(&rig.tw).families.quests;
    typed(&mut rig.tw, &family, &key).map(|row| serde_json::to_value(row).unwrap())
}

fn offer(rig: &mut Rig, actor: ObjId, npc: ObjId) -> String {
    visit(rig, actor, npc);
    let (out, queued) = command(rig, actor, &format!("+quest/offer #{}={QUEST}", npc.0));
    assert!(queued.is_empty(), "an invitation is read-only: {out}");
    out
}

fn acceptance(offer: &str) -> String {
    let token = offer
        .split("+quest/accept ")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap_or_else(|| panic!("an available invitation has an acceptance: {offer}"))
        .trim_end_matches(['.', '`']);
    format!("+quest/accept {token}")
}

fn accept(rig: &mut Rig, actor: ObjId, npc: ObjId) {
    let invitation = offer(rig, actor, npc);
    let (out, queued) = command(rig, actor, &acceptance(&invitation));
    assert_eq!(queued.len(), 1, "{out}\n{invitation}");
    land_all(rig, &queued);
    assert!(progress(rig, actor, QUEST).is_some());
}

fn step(rig: &mut Rig, actor: ObjId, npc: ObjId, source: &str, switch: &str) {
    let (out, queued) = command(rig, actor, &format!("+quest/{switch} #{}={source}", npc.0));
    assert_eq!(queued.len(), 1, "{switch}: {out}");
    land_all(rig, &queued);
}

fn refused(rig: &mut Rig, actor: ObjId, npc: ObjId, switch: &str) {
    let before = progress(rig, actor, QUEST);
    let (out, queued) = command(rig, actor, &format!("+quest/{switch} #{}={QUEST}", npc.0));
    assert!(
        queued.is_empty(),
        "unverified work cannot advance {switch}: {out}"
    );
    assert_eq!(progress(rig, actor, QUEST), before);
}

fn settle(rig: &mut Rig, contract: u64) {
    for _ in 0..8 {
        if matches!(
            survey_contract(&rig.tw, contract),
            mush_economy::ContractState::Settled { .. }
        ) {
            break;
        }
        survey_sweep(rig);
    }
    assert!(matches!(
        survey_contract(&rig.tw, contract),
        mush_economy::ContractState::Settled { .. }
    ));
    survey_sweep(rig);
}

fn relationship(rig: &Rig, npc: ObjId, actor: ObjId) -> (i16, i16) {
    let host = rig
        .tw
        .world()
        .module_state::<mush_npc::NpcHostState>()
        .unwrap();
    let relationship = host.relationships.personal(npc, actor);
    (relationship.trust, relationship.disposition)
}

/// Isolate receipt rules in adversarial tests. The authored progression itself is
/// exercised below; only its prerequisite changes, never work or payment records.
fn open_guidance(rig: &mut Rig, npc: ObjId) {
    let attr = rig
        .tw
        .world()
        .attr(npc, "quest:definition")
        .unwrap()
        .clone();
    let mut definitions = attr.as_json().unwrap().clone();
    let definition = definitions
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|definition| definition["key"] == QUEST)
        .unwrap();
    definition["prerequisite"] = json!("open");
    rig.tw
        .world_mut()
        .set_attr(
            npc,
            "quest:definition",
            mush_core::Attr::json(definitions, attr.owner),
        )
        .unwrap();
}

fn return_to_qureshi(rig: &mut Rig, actor: ObjId, hull: &HullRow, tv: ObjId, npc: ObjId) {
    act_go(rig, actor, "+tv/1 recover go");
    finish(rig, tv, actor);
    let ticket = file_and_launch(rig, actor, ObjId(hull.hull), "+ship/undock ceres");
    let _ = to_ic(rig, ticket.plan.depart_ic + ticket.plan.total_s);
    pump(rig);
    let out = run(&mut rig.tw, actor, "+ship/leave");
    assert!(out.contains("You cycle the lock out into"), "{out}");
    visit(rig, actor, npc);
}

fn sail_to_ahuna(rig: &mut Rig, actor: ObjId, hull: &HullRow) {
    board(rig, actor);
    open_ahuna(rig);
    let ticket = file_and_launch(rig, actor, ObjId(hull.hull), "+ship/undock ahuna");
    let _ = to_ic(rig, ticket.plan.depart_ic + ticket.plan.total_s);
    warm(rig);
}

fn stale_receipt_cannot_commit(rig: &mut Rig, actor: ObjId, npc: ObjId, switch: &str) {
    let before = progress(rig, actor, QUEST);
    let (out, queued) = command(rig, actor, &format!("+quest/{switch} #{}={QUEST}", npc.0));
    assert_eq!(queued.len(), 1, "{out}");
    let resolved = resolve_with_guards(rig, &queued[0]).unwrap();
    let original = commission(&mut rig.tw, "ahuna-a").unwrap();
    let mut changed = original.clone();
    changed.completed.as_mut().unwrap().ordinal += 1;
    put_commission(rig, changed);
    let query = rig
        .tw
        .engine()
        .legal_action_query(
            mush_core::PrincipalId::Object { id: actor },
            actor,
            PerceivedRef::SelfActor,
        )
        .unwrap();
    let offers = rig
        .tw
        .engine()
        .project_legal_actions(&query)
        .expect("stale optional evidence cannot break unrelated action projection");
    assert!(offers.projected_for_action(&queued[0].action).is_none());
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
        "prepare rereads the exact commission for {switch}"
    );
    assert!(land(rig, &queued[0]).is_err());
    assert_eq!(progress(rig, actor, QUEST), before);
    put_commission(rig, original);
}

/// Evict the RAM key while retaining its actual persisted document, then drive
/// the asynchronous read exactly as the world task does after a normal tick.
fn warm_cold_payment(rig: &mut Rig, actor: ObjId, npc: ObjId, contract: u64) {
    let key = mush_economy::settle_key(mush_economy::ContractId(contract));
    let document = mush_economy::idem_key(&key);
    fold_module_ops_into(&mut rig.store, rig.tw.take_module_ops());
    let economy = rig
        .tw
        .world_mut()
        .module_state_mut::<mush_economy::EconomyStore>()
        .unwrap();
    assert!(economy.forget_key_for_tests(&key));
    economy.set_key_mode_for_tests(mush_economy::KeyMode::Windowed);
    refused(rig, actor, npc, "check");
    let now = rig.tw.world().now() + 61;
    rig.tw.set_now(now);
    let mut out = mush_core::Outbox::new();
    let mut effects = mush_core::Effects::new();
    rig.tw
        .engine_mut()
        .fire_due(now * 1000, &mut out, &mut effects);
    let queued = drain(&mut rig.tw);
    land_all(rig, &queued);
    let mut fetched = false;
    for effect in effects.drain() {
        if let mush_core::Effect::StoreRead {
            module,
            actor,
            correlation,
            keys,
        } = effect
        {
            fetched |= keys.contains(&document);
            let stored = &rig
                .store
                .get(&module)
                .expect("the real persisted namespace")
                .kv;
            let mut docs = Vec::new();
            let mut absent = Vec::new();
            for key in keys {
                match stored.iter().find(|(stored_key, _)| *stored_key == key) {
                    Some((key, value)) => docs.push((key.clone(), value.clone())),
                    None => absent.push(key),
                }
            }
            let _ = rig.tw.apply(mush_core::WorldCommand::StoreReadDone {
                module,
                actor,
                correlation,
                payload: mush_core::StoreReadResult::Docs { docs, absent }.to_payload(),
            });
        }
    }
    assert!(
        fetched,
        "the tick asks for the evicted settlement's durable document"
    );
    let economy = rig
        .tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .unwrap();
    assert!(economy.snapshot().idempotency(&key).is_none());
    assert!(
        economy
            .cold_key(&key)
            .is_some_and(|value| value.record.is_some())
    );
}

#[test]
fn authored_followup_requires_the_starter_then_rewards_real_paid_work_once() {
    let mut rig = stocked();
    let (actor, hull) = prospector(&mut rig, "GuidedSurveyor", 880, "Signed Moth");
    // Buy out before leasing any rigs. Both journeys then use the ordinary
    // owned-hull path without depending on the charter's dormancy grace.
    let account = tacsim::port::credit::own_account(rig.tw.world(), actor)
        .unwrap()
        .id;
    let treasury = port_map(&rig.tw, "ceres_port").treasury.unwrap();
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@econ/mint {treasury}={} 10000000", account.0),
    );
    let registry = station_room(&rig.tw, "ceres_port", "room.c05_registry");
    teleport(&mut rig.tw, actor, registry);
    let out = run(&mut rig.tw, actor, "+coop/buyout");
    assert!(
        out.contains("The Registry writes the title transfer"),
        "{out}"
    );
    survey_sweep(&mut rig);
    assert_eq!(
        tacsim::port::coop::helm_of(rig.tw.world(), actor)
            .unwrap()
            .standing,
        tacsim::port::coop::Standing::Title
    );
    let npc = giver(&rig.tw, "quest.qureshi_first_readings");
    assert!(!offer(&mut rig, actor, npc).contains("+quest/accept "));
    onboarding_quests::accept(&mut rig, actor, npc);
    lease_one(&mut rig, actor);
    let tv = rigs_of(&rig.tw, actor)[0];
    load(&mut rig, &hull, 20, 20, 98_490);
    sail_to_ahuna(&mut rig, actor, &hull);
    act_go(&mut rig, actor, "+tv/1 scan go");
    finish(&mut rig, tv, actor);
    act_go(&mut rig, actor, "+tv/1 sound go");
    finish(&mut rig, tv, actor);
    return_to_qureshi(&mut rig, actor, &hull, tv, npc);
    step(&mut rig, actor, npc, STARTER, "check");
    step(&mut rig, actor, npc, STARTER, "check");
    step(&mut rig, actor, npc, STARTER, "finish");
    pump(&mut rig);
    assert_eq!(relationship(&rig, npc, actor), (10, 5));
    let accepted = accept_commission(&mut rig, actor, "ahuna-a");
    let payee_before = survey_money(&rig.tw, accepted.payee).0;
    accept(&mut rig, actor, npc);
    assert_eq!(
        commission(&mut rig.tw, "ahuna-a"),
        Some(accepted.clone()),
        "accepting guidance does not replace an existing commission"
    );
    refused(&mut rig, actor, npc, "check");
    load(&mut rig, &hull, 150, 200, 98_500);
    sail_to_ahuna(&mut rig, actor, &hull);
    launched(&mut rig, actor, tv);
    for cell in &CELLS[..2] {
        survey_sound(&mut rig, actor, tv, *cell, false);
    }
    assert_eq!(commission(&mut rig.tw, "ahuna-a").unwrap().covered(), 128);
    assert!(
        progress(&mut rig, actor, QUEST).unwrap()["checks"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    for cell in &CELLS[2..] {
        survey_sound(&mut rig, actor, tv, *cell, false);
    }
    let completed = commission(&mut rig.tw, "ahuna-a").unwrap();
    assert_eq!(completed.covered(), 256);
    assert_eq!(
        survey_contract(&rig.tw, accepted.contract),
        mush_economy::ContractState::Active
    );
    settle(&mut rig, accepted.contract);
    assert_eq!(
        survey_money(&rig.tw, accepted.payee).0,
        payee_before + 50_000
    );
    return_to_qureshi(&mut rig, actor, &hull, tv, npc);
    assert_eq!(
        relationship(&rig, npc, actor),
        (20, 10),
        "normal commissioned work has its own bonus"
    );
    warm_cold_payment(&mut rig, actor, npc, accepted.contract);
    stale_receipt_cannot_commit(&mut rig, actor, npc, "check");
    step(&mut rig, actor, npc, QUEST, "check");
    let checked = progress(&mut rig, actor, QUEST).unwrap();
    assert_eq!(checked["checks"][0]["proof"]["kind"], "survey_commission");
    rig = reboot(rig, &[actor]);
    pump(&mut rig);
    assert_eq!(progress(&mut rig, actor, QUEST), Some(checked));
    stale_receipt_cannot_commit(&mut rig, actor, npc, "finish");
    let paid_balance = survey_money(&rig.tw, accepted.payee).0;
    step(&mut rig, actor, npc, QUEST, "finish");
    rig = reboot(rig, &[actor]);
    pump(&mut rig);
    assert_eq!(relationship(&rig, npc, actor), (30, 15));
    assert_eq!(
        survey_money(&rig.tw, accepted.payee).0,
        paid_balance,
        "quest recognition adds no cash"
    );
    assert_eq!(commission(&mut rig.tw, "ahuna-a"), Some(completed));
    refused(&mut rig, actor, npc, "finish");
    pump(&mut rig);
    assert_eq!(relationship(&rig, npc, actor), (30, 15));
    no_refused_writes(&rig.tw);
}

#[test]
fn old_completion_and_another_captains_paid_work_do_not_count() {
    let mut rig = stocked();
    let (actor, hull) = prospector(&mut rig, "EarlierSurveyor", 879, "Earlier Moth");
    let npc = giver(&rig.tw, "quest.qureshi_first_readings");
    open_guidance(&mut rig, npc);
    let other = pilot(&mut rig, "NotTheSurveyor", 881);
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@account/open #{}=tacsim.credit", other.0),
    );
    accept(&mut rig, other, npc);
    lease_one(&mut rig, actor);
    let tv = rigs_of(&rig.tw, actor)[0];
    load(&mut rig, &hull, 20, 20, 98_510);
    let accepted = accept_commission(&mut rig, actor, "ahuna-a");
    sail_to_ahuna(&mut rig, actor, &hull);
    launched(&mut rig, actor, tv);
    for cell in CELLS {
        survey_sound(&mut rig, actor, tv, cell, false);
    }
    assert!(
        commission(&mut rig.tw, "ahuna-a")
            .unwrap()
            .completed
            .is_some()
    );
    assert_eq!(
        survey_contract(&rig.tw, accepted.contract),
        mush_economy::ContractState::Active
    );
    return_to_qureshi(&mut rig, actor, &hull, tv, npc);
    // Other board entries are available, but this completed one predates the invitation.
    accept(&mut rig, actor, npc);
    refused(&mut rig, actor, npc, "check");
    settle(&mut rig, accepted.contract);
    refused(&mut rig, actor, npc, "check");
    visit(&mut rig, other, npc);
    refused(&mut rig, other, npc, "check");
    assert_eq!(relationship(&rig, npc, other), (0, 0));
    step(&mut rig, actor, npc, QUEST, "abandon");
    assert!(progress(&mut rig, actor, QUEST).unwrap()["closed"].is_object());
    assert!(
        commission(&mut rig.tw, "ahuna-a")
            .unwrap()
            .completed
            .is_some()
    );
    no_refused_writes(&rig.tw);
}

#[test]
fn refunded_cancellation_is_not_paid_work_and_abandon_leaves_the_contract_alone() {
    let mut rig = stocked();
    let actor = pilot(&mut rig, "CommissionCancellation", 882);
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@account/open #{}=tacsim.credit", actor.0),
    );
    let npc = giver(&rig.tw, "quest.qureshi_first_readings");
    open_guidance(&mut rig, npc);
    let accepted = accept_commission(&mut rig, actor, "ahuna-a");
    accept(&mut rig, actor, npc);
    let balance = survey_money(&rig.tw, accepted.payee);
    let (out, queued) = command(&mut rig, actor, "+survey/cancel ahuna-a go");
    assert_eq!(queued.len(), 1, "{out}");
    land_all(&mut rig, &queued);
    settle(&mut rig, accepted.contract);
    assert_eq!(survey_money(&rig.tw, accepted.payee), balance);
    refused(&mut rig, actor, npc, "check");
    let cancelled = commission(&mut rig.tw, "ahuna-a").unwrap();
    assert!(cancelled.cancelled.is_some());
    step(&mut rig, actor, npc, QUEST, "abandon");
    pump(&mut rig);
    assert_eq!(commission(&mut rig.tw, "ahuna-a"), Some(cancelled));
    assert_eq!(relationship(&rig, npc, actor), (0, 0));
    assert_eq!(survey_money(&rig.tw, accepted.payee), balance);
    no_refused_writes(&rig.tw);
}

#[test]
fn acceptance_rechecks_the_finite_board_and_hides_an_unavailable_invitation() {
    let mut rig = stocked();
    let actor = pilot(&mut rig, "WaitingSurveyor", 883);
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@account/open #{}=tacsim.credit", actor.0),
    );
    let npc = giver(&rig.tw, "quest.qureshi_first_readings");
    open_guidance(&mut rig, npc);
    let invitation = offer(&mut rig, actor, npc);
    let (out, queued) = command(&mut rig, actor, &acceptance(&invitation));
    assert_eq!(queued.len(), 1, "{out}");
    let resolved = resolve_with_guards(&rig, &queued[0]).unwrap();
    for (index, offer) in ["ahuna-a", "kerwan-f", "juling-k", "occator-p"]
        .into_iter()
        .enumerate()
    {
        let owner = pilot(
            &mut rig,
            &format!("WorkingSurveyor{index}"),
            884 + i64::try_from(index).unwrap(),
        );
        let _ = run(
            &mut rig.tw,
            GOD,
            &format!("@account/open #{}=tacsim.credit", owner.0),
        );
        accept_commission(&mut rig, owner, offer);
        if index == 0 {
            let sibling = pilot(&mut rig, "SameAccountSurveyor", 889);
            {
                let mut accounts = rig.oracle.0.lock().unwrap();
                let owner_account = accounts[&owner];
                accounts.insert(sibling, owner_account);
            }
            let _ = run(
                &mut rig.tw,
                GOD,
                &format!("@account/open #{}=tacsim.credit", sibling.0),
            );
            assert!(
                !self::offer(&mut rig, sibling, npc).contains("+quest/accept "),
                "another character on the same account cannot promise a second commission"
            );
        }
    }
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
        "acceptance rereads authoritative commission availability"
    );
    assert!(land(&mut rig, &queued[0]).is_err());
    assert!(progress(&mut rig, actor, QUEST).is_none());
    assert!(!offer(&mut rig, actor, npc).contains("+quest/accept "));
    no_refused_writes(&rig.tw);
}

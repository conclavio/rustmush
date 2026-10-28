//! Choices and closures are durable player decisions; the work remains ordinary gameplay.
use super::onboarding_quests::{giver, pump, visit};
use super::*;
use serde_json::{Value, json};

const FIRST: &str = "tacsim.passengers.stickney/quest.leila_second_reading";
const TRADE: &str = "tacsim.quests.leila_followup/quest.leila_trade";
const SURVEY: &str = "tacsim.quests.leila_followup/quest.leila_survey";

fn key(source: &str, actor: ObjId, booking: Option<u64>) -> Value {
    match booking {
        Some(booking) => json!({"source":source,"booking":booking}),
        None => json!({"source":source,"actor":actor}),
    }
}

fn progress(rig: &mut Rig, source: &str, actor: ObjId, booking: Option<u64>) -> Option<Value> {
    let family = handles(&rig.tw).families.quests;
    let key = serde_json::from_value(key(source, actor, booking)).unwrap();
    typed(&mut rig.tw, &family, &key).map(|row| serde_json::to_value(row).unwrap())
}

fn replace_receipt(rig: &mut Rig, key: Value, row: Value) {
    let family = handles(&rig.tw).families.quests;
    let key = serde_json::from_value(key).unwrap();
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

fn acceptance(offer: &str) -> String {
    let token = offer
        .split("+quest/accept ")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap_or_else(|| panic!("offer must contain an explicit acceptance: {offer}"))
        .trim_end_matches(['.', '`']);
    format!("+quest/accept {token}")
}

fn offer(rig: &mut Rig, actor: ObjId, npc: ObjId, source: &str) -> String {
    let (out, queued) = command(rig, actor, &format!("+quest/offer #{}={source}", npc.0));
    assert!(
        queued.is_empty(),
        "offers cannot advance the journal: {out}"
    );
    out
}

fn accept(rig: &mut Rig, actor: ObjId, npc: ObjId, source: &str) {
    visit(rig, actor, npc);
    let offer = offer(rig, actor, npc, source);
    let (out, queued) = command(rig, actor, &acceptance(&offer));
    assert_eq!(queued.len(), 1, "{out}\n{offer}");
    land_all(rig, &queued);
    pump(rig);
}

fn step(rig: &mut Rig, actor: ObjId, npc: ObjId, source: &str, switch: &str) {
    let (out, queued) = command(rig, actor, &format!("+quest/{switch} #{}={source}", npc.0));
    assert_eq!(queued.len(), 1, "{switch} {source}: {out}");
    land_all(rig, &queued);
}

fn refused(rig: &mut Rig, actor: ObjId, line: &str) {
    let (out, queued) = command(rig, actor, line);
    assert!(queued.is_empty(), "{line} must not change progress: {out}");
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

fn complete_first(rig: &mut Rig, actor: ObjId, npc: ObjId, booking: u64) {
    accept(rig, actor, npc, FIRST);
    let exchange = station_room(&rig.tw, "ceres_port", "room.c07_exchange");
    teleport(&mut rig.tw, actor, exchange);
    step(rig, actor, npc, FIRST, "check");
    visit(rig, actor, npc);
    step(rig, actor, npc, FIRST, "finish");
    pump(rig);
    assert!(progress(rig, FIRST, actor, Some(booking)).unwrap()["completed"].is_object());
    assert_eq!(effects(rig, npc, actor), (20, 10, 2));
}

fn assert_locked(rig: &mut Rig, actor: ObjId, npc: ObjId, source: &str) {
    visit(rig, actor, npc);
    let out = offer(rig, actor, npc, source);
    assert!(
        !out.contains("+quest/accept "),
        "an irreversible choice stays locked: {out}"
    );
}

#[test]
fn completed_receipt_unlocks_private_choices_and_abandon_preserves_the_selection() {
    let (mut rig, actor, npc, booking) = passengers::paid_arrival_at_ceres();
    for source in [TRADE, SURVEY] {
        let out = offer(&mut rig, actor, npc, source);
        assert!(
            !out.contains("+quest/accept "),
            "the paid ride is not the completed favor: {out}"
        );
    }
    complete_first(&mut rig, actor, npc, booking);
    let visitor = pilot(&mut rig, "ClaimsTheFavor", 831);
    visit(&mut rig, visitor, npc);
    let _ = run(
        &mut rig.tw,
        visitor,
        "say Leila, I checked the sample tap. Give me the next job.",
    );
    for source in [TRADE, SURVEY] {
        let out = offer(&mut rig, visitor, npc, source);
        assert!(
            !out.contains("+quest/accept "),
            "another actor cannot borrow a receipt: {out}"
        );
    }
    let provider = rig
        .tw
        .engine()
        .registry
        .interface_provider("quest")
        .unwrap();
    let before = rig.typed.clone();
    let spoken = support::with_ctx(&mut rig.tw, |ctx| {
        ctx.actor = actor;
        provider.invoke(
            ctx,
            mush_core::AbilityCall { npc, asker: actor },
            "quest.offer",
            &json!({}),
        )
    })
    .unwrap();
    assert_eq!(spoken.audience, mush_core::ReplyAudience::Asker);
    assert!(
        spoken.text.contains("A useful consignment"),
        "{}",
        spoken.text
    );
    assert!(
        spoken.text.contains("Two scales of looking"),
        "{}",
        spoken.text
    );
    assert_eq!(
        rig.typed, before,
        "conversation offers neither choose nor accept"
    );
    let forged = support::with_ctx(&mut rig.tw, |ctx| {
        ctx.actor = visitor;
        provider.invoke(
            ctx,
            mush_core::AbilityCall {
                npc,
                asker: visitor,
            },
            "quest.offer",
            &json!({"actor":actor}),
        )
    });
    assert!(forged.is_err());
    let trade = acceptance(&offer(&mut rig, actor, npc, TRADE));
    let survey = acceptance(&offer(&mut rig, actor, npc, SURVEY));

    // A previously hydrated conversation view is not authority to unlock work.
    let first = progress(&mut rig, FIRST, actor, Some(booking)).unwrap();
    let mut incomplete = first.clone();
    incomplete["completed"] = Value::Null;
    incomplete["revision"] = json!(1 + incomplete["checks"].as_array().unwrap().len());
    replace_receipt(&mut rig, key(FIRST, actor, Some(booking)), incomplete);
    refused(&mut rig, actor, &trade);
    assert!(progress(&mut rig, TRADE, actor, None).is_none());
    replace_receipt(&mut rig, key(FIRST, actor, Some(booking)), first);
    pump(&mut rig);

    let (out, queued) = command(&mut rig, actor, &trade);
    assert_eq!(queued.len(), 1, "{out}");
    land_all(&mut rig, &queued);
    let selected = progress(&mut rig, TRADE, actor, None).unwrap();
    assert_eq!(
        selected["prerequisite_receipt"],
        key(FIRST, actor, Some(booking))
    );
    assert_eq!(selected["definition"]["choice"]["path"], "trade");
    refused(&mut rig, actor, &survey);
    assert!(progress(&mut rig, SURVEY, actor, None).is_none());
    let remote = station_room(&rig.tw, "ceres_port", "room.c07_exchange");
    teleport(&mut rig.tw, actor, remote);
    step(&mut rig, actor, npc, TRADE, "abandon");
    let closed = progress(&mut rig, TRADE, actor, None).unwrap();
    assert_eq!(closed["closed"]["kind"], "abandoned");
    assert!(closed["completed"].is_null());
    rig = reboot(rig, &[actor, visitor]);
    pump(&mut rig);
    assert_eq!(progress(&mut rig, TRADE, actor, None), Some(closed));
    assert_eq!(
        effects(&rig, npc, actor),
        (20, 10, 2),
        "abandon earns no work reward"
    );
    assert_locked(&mut rig, actor, npc, SURVEY);
    refused(&mut rig, actor, &survey);
    let ma = giver(&rig.tw, "quest.ma_connections");
    onboarding_quests::accept(&mut rig, actor, ma);
    assert!(
        onboarding_quests::progress(&mut rig, actor, ma).is_some(),
        "the ordinary starter remains independent"
    );
    no_refused_writes(&rig.tw);
}

#[test]
fn declining_one_path_allows_the_other_and_real_trade_rewards_only_once() {
    let (mut rig, actor, npc, booking) = owned_arrival(false);
    complete_first(&mut rig, actor, npc, booking);
    let survey = acceptance(&offer(&mut rig, actor, npc, SURVEY));
    let decline = survey.replacen("+quest/accept", "+quest/decline", 1);
    let (out, queued) = command(&mut rig, actor, &decline);
    assert_eq!(queued.len(), 1, "{out}");
    land_all(&mut rig, &queued);
    let declined = progress(&mut rig, SURVEY, actor, None).unwrap();
    assert_eq!(declined["closed"]["kind"], "declined");
    assert!(declined["completed"].is_null());
    rig = reboot(rig, &[actor]);
    pump(&mut rig);
    refused(&mut rig, actor, &survey);
    accept(&mut rig, actor, npc, TRADE);
    assert_eq!(effects(&rig, npc, actor), (20, 10, 2));
    let hull = tacsim::port::coop::helm_of(rig.tw.world(), actor)
        .expect("the purchased hull stays under its owner's command after restart")
        .hull;
    board(&mut rig, actor);
    let outbound = file_and_launch(&mut rig, actor, hull, "+ship/undock stickney");
    passengers::arrive(&mut rig, &outbound, "stickney_port");
    let out = run(&mut rig.tw, actor, "+ship/leave");
    assert!(out.contains("You cycle the lock out into"), "{out}");
    let purchase = onboarding_trade::buy(
        &mut rig,
        actor,
        "stickney_port",
        "stickney_yards",
        "industrial_parts",
    );
    assert!(purchase.settled.is_some());
    // This is the fourth interplanetary leg after the passenger journey.
    // Buy and draw real propellant at Kasei's local exchange before returning.
    let fuel =
        onboarding_trade::buy_quantity(&mut rig, actor, "stickney_port", "kasei", "propellant", 60);
    assert!(fuel.drawn_at.is_some() && fuel.settled.is_some());
    rig = reboot(rig, &[actor]);
    pump(&mut rig);
    let gallery = station_room(&rig.tw, "stickney_port", "room.p01_berth_gallery");
    teleport(&mut rig.tw, actor, gallery);
    let out = run(&mut rig.tw, actor, "+ship/board");
    assert!(out.contains("You cycle the lock into"), "{out}");
    let inbound = file_and_launch(&mut rig, actor, hull, "+ship/undock ceres");
    passengers::arrive(&mut rig, &inbound, "ceres_port");
    let out = run(&mut rig.tw, actor, "+ship/leave");
    assert!(out.contains("You cycle the lock out into"), "{out}");
    let (sale, queued) = onboarding_trade::offer_sale(&mut rig, actor, "coop", "industrial_parts");
    refused(&mut rig, actor, &format!("+quest/check #{}={TRADE}", npc.0));
    land_all(&mut rig, &queued);
    refused(&mut rig, actor, &format!("+quest/check #{}={TRADE}", npc.0));
    onboarding_trade::settle_trade(&mut rig, sale.id);
    step(&mut rig, actor, npc, TRADE, "check");
    let row = progress(&mut rig, TRADE, actor, None).unwrap();
    assert_eq!(row["checks"][0]["proof"]["kind"], "trade");
    refused(
        &mut rig,
        actor,
        &format!("+quest/finish #{}={TRADE}", npc.0),
    );
    visit(&mut rig, actor, npc);
    step(&mut rig, actor, npc, TRADE, "finish");
    assert_eq!(
        effects(&rig, npc, actor),
        (20, 10, 2),
        "completion precedes reward reconciliation"
    );
    rig = reboot(rig, &[actor]);
    pump(&mut rig);
    assert_eq!(effects(&rig, npc, actor), (30, 15, 3));
    let memories = rig
        .tw
        .world()
        .module_state::<mush_npc::NpcHostState>()
        .unwrap()
        .personal_memory
        .record_for(npc)
        .unwrap()
        .clone();
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@name #{}=Renamed Trader", actor.0),
    );
    refused(
        &mut rig,
        actor,
        &format!("+quest/finish #{}={TRADE}", npc.0),
    );
    pump(&mut rig);
    assert_eq!(effects(&rig, npc, actor), (30, 15, 3));
    assert_eq!(
        rig.tw
            .world()
            .module_state::<mush_npc::NpcHostState>()
            .unwrap()
            .personal_memory
            .record_for(npc),
        Some(&memories)
    );
    assert_eq!(progress(&mut rig, SURVEY, actor, None), Some(declined));
    assert_locked(&mut rig, actor, npc, SURVEY);
    no_refused_writes(&rig.tw);
}

/// These follow-ups make several long voyages and wait ashore for receipts.
/// Buy out the hull through the real settlement path before sailing, so the
/// Co-op's ordinary seven-day dormancy recall is not this fixture's clock.
/// Refit before loading too: provisions correctly prevent refitting later.
fn owned_arrival(prospecting: bool) -> (Rig, ObjId, ObjId, u64) {
    let mut rig = stocked();
    let (actor, hull) = if prospecting {
        prospector(&mut rig, "SurveyingCaptain", 832, "Survey Passage")
    } else {
        passengers::charter(&mut rig, "TradingCaptain", 836, "Trade Passage")
    };
    let account = tacsim::port::credit::own_account(rig.tw.world(), actor)
        .expect("the captain's credit account")
        .id;
    let treasury = port_map(&rig.tw, "ceres_port").treasury.unwrap();
    let funded = run(
        &mut rig.tw,
        GOD,
        &format!("@econ/mint {treasury}={} 10000000", account.0),
    );
    assert!(!funded.contains("Usage"), "{funded}");
    let registry = station_room(&rig.tw, "ceres_port", "room.c05_registry");
    teleport(&mut rig.tw, actor, registry);
    let bought = run(&mut rig.tw, actor, "+coop/buyout");
    assert!(
        bought.contains("The Registry writes the title transfer"),
        "{bought}"
    );
    survey_sweep(&mut rig);
    assert_eq!(
        tacsim::port::coop::helm_of(rig.tw.world(), actor),
        Some(tacsim::port::coop::Helm {
            hull: ObjId(hull.hull),
            standing: tacsim::port::coop::Standing::Title,
        })
    );
    load(&mut rig, &hull, 150, 200, 98_200);
    board(&mut rig, actor);
    let outbound = file_and_launch(&mut rig, actor, ObjId(hull.hull), "+ship/undock stickney");
    passengers::arrive(&mut rig, &outbound, "stickney_port");
    let out = run(&mut rig.tw, actor, "+ship/leave");
    assert!(out.contains("You cycle the lock out into"), "{out}");
    let npc = mush_atlas::travel::latched_object(
        rig.tw.world(),
        &mush_atlas::RecordKey::new(
            cid("tacsim.passengers.stickney"),
            cid("npc.leila_mensah_kovacs"),
        ),
    )
    .unwrap();
    visit(&mut rig, actor, npc);
    let (booking, _) = passengers::book(&mut rig, actor, npc, "350");
    passengers::invite(&mut rig, actor, npc, booking);
    let gallery = station_room(&rig.tw, "stickney_port", "room.p01_berth_gallery");
    teleport(&mut rig.tw, actor, gallery);
    let out = run(&mut rig.tw, actor, "+ship/board");
    assert!(out.contains("You cycle the lock into"), "{out}");
    let inbound = file_and_launch(&mut rig, actor, ObjId(hull.hull), "+ship/undock ceres");
    passengers::arrive(&mut rig, &inbound, "ceres_port");
    let out = run(&mut rig.tw, actor, "+ship/leave");
    assert!(out.contains("You cycle the lock out into"), "{out}");
    for _ in 0..8 {
        survey_sweep(&mut rig);
        if matches!(
            survey_contract(&rig.tw, booking),
            mush_economy::ContractState::Settled { .. }
        ) {
            break;
        }
    }
    pump(&mut rig);
    assert!(matches!(
        survey_contract(&rig.tw, booking),
        mush_economy::ContractState::Settled { .. }
    ));
    visit(&mut rig, actor, npc);
    (rig, actor, npc, booking)
}

#[test]
fn survey_path_uses_fresh_real_readings_and_keeps_qureshis_starter_independent() {
    let (mut rig, actor, npc, booking) = owned_arrival(true);
    complete_first(&mut rig, actor, npc, booking);
    accept(&mut rig, actor, npc, SURVEY);
    let qureshi = giver(&rig.tw, "quest.qureshi_first_readings");
    onboarding_quests::accept(&mut rig, actor, qureshi);
    refused(
        &mut rig,
        actor,
        &format!("+quest/check #{}={SURVEY}", npc.0),
    );
    lease_one(&mut rig, actor);
    let tv = rigs_of(&rig.tw, actor)[0];
    let hull = tacsim::port::coop::helm_of(rig.tw.world(), actor)
        .expect("the paid title permits a later survey voyage")
        .hull;
    board(&mut rig, actor);
    open_ahuna(&mut rig);
    let ticket = file_and_launch(&mut rig, actor, hull, "+ship/undock ahuna");
    let _ = to_ic(&mut rig, ticket.plan.depart_ic + ticket.plan.total_s);
    warm(&mut rig);
    for line in ["+tv/1 scan go", "+tv/1 sound go", "+tv/1 recover go"] {
        act_go(&mut rig, actor, line);
        finish(&mut rig, tv, actor);
        assert_eq!(
            record_of(&mut rig.tw, tv).unwrap().last.unwrap().result,
            ActResult::Done
        );
    }
    let ticket = file_and_launch(&mut rig, actor, hull, "+ship/undock ceres");
    let _ = to_ic(&mut rig, ticket.plan.depart_ic + ticket.plan.total_s);
    pump(&mut rig);
    let out = run(&mut rig.tw, actor, "+ship/leave");
    assert!(out.contains("You cycle the lock out into"), "{out}");
    visit(&mut rig, actor, qureshi);
    rig = reboot(rig, &[actor]);
    pump(&mut rig);
    step(&mut rig, actor, npc, SURVEY, "check");
    step(&mut rig, actor, npc, SURVEY, "check");
    let row = progress(&mut rig, SURVEY, actor, None).unwrap();
    assert_eq!(row["checks"].as_array().unwrap().len(), 2);
    assert_eq!(row["definition"]["choice"]["path"], "survey");
    for _ in 0..2 {
        onboarding_quests::step(&mut rig, actor, qureshi, "check");
    }
    onboarding_quests::step(&mut rig, actor, qureshi, "finish");
    visit(&mut rig, actor, npc);
    step(&mut rig, actor, npc, SURVEY, "finish");
    rig = reboot(rig, &[actor]);
    pump(&mut rig);
    assert_eq!(effects(&rig, npc, actor), (30, 15, 3));
    assert_eq!(effects(&rig, qureshi, actor), (10, 5, 1));
    assert!(progress(&mut rig, SURVEY, actor, None).unwrap()["completed"].is_object());
    assert_locked(&mut rig, actor, npc, TRADE);
    refused(
        &mut rig,
        actor,
        &format!("+quest/finish #{}={SURVEY}", npc.0),
    );
    pump(&mut rig);
    assert_eq!(effects(&rig, npc, actor), (30, 15, 3));
    no_refused_writes(&rig.tw);
}

#[test]
fn independent_quests_on_one_giver_require_explicit_selection() {
    let mut rig = stocked();
    let actor = pilot(&mut rig, "TwoFavors", 833);
    let npc = giver(&rig.tw, "quest.oduya_first_hull");
    let first = rig
        .tw
        .world()
        .attr(npc, "quest:definition")
        .unwrap()
        .as_json()
        .unwrap()
        .clone();
    let first_key = first["key"].as_str().unwrap().to_owned();
    let mut second = first.clone();
    let second_key = "test.quests/another_hull_review";
    second["key"] = json!(second_key);
    second["title"] = json!("A separate hull review");
    rig.tw
        .world_mut()
        .set_attr(
            npc,
            "quest:definition",
            mush_core::Attr::json(json!([first, second]), GOD),
        )
        .unwrap();
    visit(&mut rig, actor, npc);
    let (offers, queued) = command(&mut rig, actor, &format!("+quest/offer #{}", npc.0));
    assert!(queued.is_empty());
    assert_eq!(offers.matches("+quest/accept ").count(), 2, "{offers}");
    accept(&mut rig, actor, npc, &first_key);
    accept(&mut rig, actor, npc, second_key);
    let out = run(&mut rig.tw, actor, "+coop/enrol Two Reviews");
    assert!(out.contains("now the Two Reviews"), "{out}");
    for switch in ["check", "finish", "abandon"] {
        let (out, queued) = command(&mut rig, actor, &format!("+quest/{switch} #{}", npc.0));
        assert!(
            queued.is_empty(),
            "ambiguous {switch} must not choose for the player: {out}"
        );
        assert!(
            out.contains(&first_key) && out.contains(second_key),
            "give usable selectors: {out}"
        );
    }
    step(&mut rig, actor, npc, &first_key, "check");
    assert!(
        progress(&mut rig, second_key, actor, None).unwrap()["checks"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    step(&mut rig, actor, npc, &first_key, "finish");
    let (out, queued) = command(&mut rig, actor, &format!("+quest/abandon #{}", npc.0));
    assert_eq!(
        queued.len(),
        1,
        "the sole remaining active favor is unambiguous: {out}"
    );
    land_all(&mut rig, &queued);
    rig = reboot(rig, &[actor]);
    pump(&mut rig);
    assert!(progress(&mut rig, &first_key, actor, None).unwrap()["completed"].is_object());
    assert_eq!(
        progress(&mut rig, second_key, actor, None).unwrap()["closed"]["kind"],
        "abandoned"
    );
    assert_eq!(effects(&rig, npc, actor), (10, 5, 1));
    no_refused_writes(&rig.tw);
}

#[test]
fn unavailable_traveller_and_absent_giver_do_not_trap_an_accepted_quest() {
    let mut rig = stocked();
    let actor = pilot(&mut rig, "ChangesPlans", 834);
    let ma = giver(&rig.tw, "quest.ma_connections");
    let source = "tacsim.quests.first_steps/quest.ma_connections";
    onboarding_quests::accept(&mut rig, actor, ma);
    let nadia = mush_atlas::travel::latched_object(
        rig.tw.world(),
        &mush_atlas::RecordKey::new(cid("tacsim.passengers.ceres"), cid("npc.nadia_okafor_sato")),
    )
    .unwrap();
    let remote = station_room(&rig.tw, "ceres_port", "room.c07_exchange");
    teleport(&mut rig.tw, nadia, remote);
    teleport(&mut rig.tw, ma, remote);
    refused(&mut rig, actor, &format!("+quest/check #{}={source}", ma.0));
    step(&mut rig, actor, ma, source, "abandon");
    let abandoned = progress(&mut rig, source, actor, None).unwrap();
    assert_eq!(abandoned["closed"]["kind"], "abandoned");
    rig = reboot(rig, &[actor]);
    pump(&mut rig);
    assert_eq!(progress(&mut rig, source, actor, None), Some(abandoned));
    assert_eq!(effects(&rig, ma, actor), (0, 0, 0));
    for switch in ["check", "finish", "abandon"] {
        refused(
            &mut rig,
            actor,
            &format!("+quest/{switch} #{}={source}", ma.0),
        );
    }
    no_refused_writes(&rig.tw);
}

#[test]
fn adding_followups_preserves_a_completed_historical_single_quest_and_its_passenger() {
    let (mut rig, actor, npc, booking) =
        passengers::paid_arrival_at_ceres_from(stocked_with_passenger(true, true));
    // Load the current catalog, which must wait for the explicitly approved
    // passenger v1 -> v2 upgrade before it can seed the later quest pack.
    rig = reboot(rig, &[actor]);
    let plan = run(
        &mut rig.tw,
        GOD,
        "@atlas/pack plan tacsim.passengers.stickney",
    );
    let fingerprint = plan
        .split("@atlas/pack apply ")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap_or_else(|| panic!("passenger upgrade needs an explicit plan: {plan}"));
    let applied = run(
        &mut rig.tw,
        GOD,
        &format!("@atlas/pack apply {fingerprint}"),
    );
    assert!(
        applied.contains("Applied tacsim.passengers.stickney@2:"),
        "{applied}"
    );
    assert!(
        rig.tw
            .world()
            .module_state::<mush_content::ContentState>()
            .unwrap()
            .latches
            .applied(&cid(tacsim::quest::packs::LEILA_PACK))
            .is_none()
    );
    assert!(
        rig.tw
            .world()
            .attr(npc, "quest:definition")
            .unwrap()
            .as_json()
            .unwrap()
            .is_object()
    );
    complete_first(&mut rig, actor, npc, booking);
    let historical = progress(&mut rig, FIRST, actor, Some(booking)).unwrap();
    assert!(historical.get("prerequisite_receipt").is_none());
    assert!(historical.get("closed").is_none());
    let passage = passengers::booking(&mut rig, booking);
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
    let count = rig.tw.world().iter().count();
    rig = reboot(rig, &[actor]);
    pump(&mut rig);
    assert!(
        rig.tw
            .world()
            .module_state::<mush_content::ContentState>()
            .unwrap()
            .latches
            .applied(&cid(tacsim::quest::packs::LEILA_PACK))
            .is_some()
    );
    assert_eq!(
        rig.tw.world().iter().count(),
        count,
        "the new pack adds no replacement NPC"
    );
    assert_eq!(rig.tw.world().get(npc).unwrap().location, location);
    assert_eq!(passengers::booking(&mut rig, booking), passage);
    assert_eq!(
        progress(&mut rig, FIRST, actor, Some(booking)),
        Some(historical)
    );
    assert_eq!(
        rig.tw
            .world()
            .module_state::<mush_npc::NpcHostState>()
            .unwrap()
            .personal_memory
            .record_for(npc),
        Some(&memories)
    );
    assert_eq!(effects(&rig, npc, actor), (20, 10, 2));
    for source in [TRADE, SURVEY] {
        let offered = offer(&mut rig, actor, npc, source);
        assert!(
            offered.contains("+quest/accept "),
            "the old completion unlocks the new work: {offered}"
        );
    }
    assert!(progress(&mut rig, TRADE, actor, None).is_none());
    assert!(progress(&mut rig, SURVEY, actor, None).is_none());
    no_refused_writes(&rig.tw);
}

#[test]
fn large_npc_quest_catalog_returns_complete_private_commands_and_exact_selectors() {
    let mut rig = stocked();
    let actor = pilot(&mut rig, "ManyInvitations", 835);
    let npc = giver(&rig.tw, "quest.oduya_first_hull");
    let original = rig
        .tw
        .world()
        .attr(npc, "quest:definition")
        .unwrap()
        .as_json()
        .unwrap()
        .clone();
    let sources: Vec<_> = (0..8)
        .map(|n| format!("test.quests/catalog_favor_{n}"))
        .collect();
    let definitions: Vec<_> = sources
        .iter()
        .enumerate()
        .map(|(n, source)| {
            let mut definition = original.clone();
            definition["key"] = json!(source);
            definition["title"] = json!(format!("Registry favor {n}"));
            definition["introduction"] = json!("Explain the optional work carefully. ".repeat(50));
            definition["checkpoints"][0]["instructions"] =
                json!("Read the actual charter papers before returning. ".repeat(20));
            if n == 7 {
                // One valid individual offer also exceeds the tool budget.
                // It must link to complete terms, never truncate its token.
                let mut extra = definition["checkpoints"][0].clone();
                extra["id"] = json!("another_review");
                definition["checkpoints"]
                    .as_array_mut()
                    .unwrap()
                    .push(extra);
            }
            definition
        })
        .collect();
    rig.tw
        .world_mut()
        .set_attr(
            npc,
            "quest:definition",
            mush_core::Attr::json(json!(definitions), GOD),
        )
        .unwrap();
    visit(&mut rig, actor, npc);
    pump(&mut rig);
    let provider = rig
        .tw
        .engine()
        .registry
        .interface_provider("quest")
        .unwrap();
    let read = |rig: &mut Rig, ability: &str, input: Value| {
        support::with_ctx(&mut rig.tw, |ctx| {
            ctx.actor = actor;
            provider.invoke(
                ctx,
                mush_core::AbilityCall { npc, asker: actor },
                ability,
                &input,
            )
        })
        .unwrap()
    };
    let before = rig.typed.clone();
    let (full, queued) = command(&mut rig, actor, &format!("+quest/offer #{}", npc.0));
    assert!(queued.is_empty());
    assert!(full.len() > mush_core::MAX_INTERFACE_REPLY_BYTES);
    let summary = read(&mut rig, "quest.offer", json!({}));
    assert_eq!(summary.audience, mush_core::ReplyAudience::Asker);
    assert!(summary.text.len() <= mush_core::MAX_INTERFACE_REPLY_BYTES);
    assert!(
        !summary.text.contains("+quest/accept "),
        "summaries carry no partial consent token"
    );
    assert!(!summary.text.contains('…') && !summary.text.contains("..."));
    for source in &sources {
        assert!(
            summary
                .text
                .contains(&format!("+quest/offer #{}={source}\n", npc.0)),
            "complete source-specific command: {}",
            summary.text
        );
    }
    let selected = read(&mut rig, "quest.offer", json!({"quest":sources[0]}));
    assert_eq!(selected.audience, mush_core::ReplyAudience::Asker);
    assert!(selected.text.len() <= mush_core::MAX_INTERFACE_REPLY_BYTES);
    assert!(
        selected.text.starts_with("Registry favor 0\n"),
        "{}",
        selected.text
    );
    assert!(!selected.text.contains("Registry favor 1"));
    let consent = acceptance(&selected.text);
    let (oversized, queued) = command(
        &mut rig,
        actor,
        &format!("+quest/offer #{}={}", npc.0, sources[7]),
    );
    assert!(queued.is_empty());
    assert!(oversized.len() > mush_core::MAX_INTERFACE_REPLY_BYTES);
    let selected_large = read(&mut rig, "quest.offer", json!({"quest":sources[7]}));
    assert_eq!(selected_large.audience, mush_core::ReplyAudience::Asker);
    assert!(selected_large.text.len() <= mush_core::MAX_INTERFACE_REPLY_BYTES);
    assert!(
        selected_large
            .text
            .contains(&format!("+quest/offer #{}={}\n", npc.0, sources[7]))
    );
    assert!(!selected_large.text.contains("+quest/accept "));
    assert!(!selected_large.text.contains('…') && !selected_large.text.contains("..."));
    assert_eq!(
        rig.typed, before,
        "every tool read leaves the journal unchanged"
    );

    let (out, queued) = command(&mut rig, actor, &consent);
    assert_eq!(
        queued.len(),
        1,
        "the selected tool returned an intact acceptance token: {out}"
    );
    land_all(&mut rig, &queued);
    for source in &sources[1..] {
        accept(&mut rig, actor, npc, source);
    }
    let (full_journal, queued) = command(&mut rig, actor, &format!("+quest #{}", npc.0));
    assert!(queued.is_empty());
    assert!(full_journal.len() > mush_core::MAX_INTERFACE_REPLY_BYTES);
    let journal = read(&mut rig, "quest.status", json!({}));
    assert_eq!(journal.audience, mush_core::ReplyAudience::Asker);
    assert!(journal.text.len() <= mush_core::MAX_INTERFACE_REPLY_BYTES);
    assert!(!journal.text.contains('…') && !journal.text.contains("..."));
    for source in &sources {
        assert!(
            journal
                .text
                .contains(&format!("Read: +quest #{}={source}\n", npc.0)),
            "{}",
            journal.text
        );
    }
    let single = read(&mut rig, "quest.status", json!({"quest":sources[0]}));
    assert_eq!(single.audience, mush_core::ReplyAudience::Asker);
    assert!(single.text.contains("Registry favor 0"));
    assert!(!single.text.contains("Registry favor 1"));
    assert_eq!(effects(&rig, npc, actor), (0, 0, 0));
    no_refused_writes(&rig.tw);
}

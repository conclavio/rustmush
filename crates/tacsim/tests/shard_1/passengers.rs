//! Real passenger agreements, occupancy, voyages and durable settlement.
//! The parent supplies the production roster and authoritative commit fixture.

use super::*;
use mush_core::ObjKind;
use serde_json::{Value, json};

const PASSENGER_PACK: &str = "tacsim.passengers.stickney";
const PASSENGER_ENTRY: &str = "npc.leila_mensah_kovacs";

fn leila(tw: &TestWorld) -> ObjId {
    mush_atlas::travel::latched_object(
        tw.world(),
        &mush_atlas::RecordKey::new(cid(PASSENGER_PACK), cid(PASSENGER_ENTRY)),
    )
    .expect("the authored traveller is latched")
}

fn booking(rig: &mut Rig, contract: u64) -> Value {
    let h = handles(&rig.tw);
    let key = serde_json::from_value(json!(contract)).expect("booking key");
    let row = typed(&mut rig.tw, &h.families.passengers, &key).expect("durable passenger booking");
    serde_json::to_value(row).expect("booking projection")
}

fn next_contract(tw: &TestWorld) -> u64 {
    tw.world()
        .module_state::<mush_economy::EconomyStore>()
        .unwrap()
        .peek_next_contract()
}

fn pump(rig: &mut Rig, count: usize) {
    for _ in 0..count {
        let (_, queued) = fire(rig);
        for submit in queued {
            if submit.action.key().id.as_str() == "tacsim_passenger" {
                land(rig, &submit).expect("passenger transition commits");
            } else {
                // An unrelated stock/market attempt may have become redundant
                // while the coarse clock advanced through the voyage.
                let _ = land(rig, &submit);
            }
        }
    }
}

fn charter(rig: &mut Rig, name: &str, seed: i64, ship: &str) -> (ObjId, HullRow) {
    let captain = pilot(rig, name, seed);
    let out = run(&mut rig.tw, captain, &format!("+coop/enrol {ship}"));
    assert!(out.contains(&format!("now the {ship}")), "{out}");
    let hull = tacsim::port::live_charter(rig.tw.world(), captain)
        .expect("charter")
        .hull;
    let row = port_map(&rig.tw, "ceres_port")
        .hulls
        .into_iter()
        .find(|row| row.hull == hull.0)
        .unwrap();
    (captain, row)
}

fn arrive(rig: &mut Rig, ticket: &Ticket, expected: &str) {
    let _ = to_ic(rig, ticket.plan.depart_ic + ticket.plan.total_s);
    pump(rig, 3);
    assert!(
        matches!(tacsim::port::ship::berth_of(rig.tw.world(), ticket.hull),
        tacsim::port::ship::Berth::Docked { map, .. } if map.entry.as_str() == expected)
    );
}

fn at_stickney() -> (Rig, ObjId, HullRow, ObjId) {
    let mut rig = stocked();
    let (captain, row) = charter(&mut rig, "Captain", 201, "Passenger Moth");
    load(&mut rig, &row, 150, 200, 90_000);
    board(&mut rig, captain);
    let outbound = file_and_launch(&mut rig, captain, ObjId(row.hull), "+ship/undock stickney");
    arrive(&mut rig, &outbound, "stickney_port");
    let out = run(&mut rig.tw, captain, "+ship/leave");
    assert!(out.contains("You cycle the lock out into"), "{out}");
    let npc = leila(&rig.tw);
    let bar = station_room(&rig.tw, "stickney_port", "room.p10_shift_board");
    assert_eq!(rig.tw.world().get(npc).unwrap().location, Some(bar));
    teleport(&mut rig.tw, captain, bar);
    pump(&mut rig, 2);
    (rig, captain, row, npc)
}

fn acceptance(quote: &str) -> String {
    let token = quote
        .split("+passage/accept ")
        .nth(1)
        .and_then(|tail| tail.split_whitespace().next())
        .unwrap_or_else(|| panic!("quote carries exact acceptance command: {quote}"))
        .trim_end_matches('.');
    format!("+passage/accept {token}")
}

fn book(rig: &mut Rig, captain: ObjId, npc: ObjId, fare: &str) -> (u64, Value) {
    let before = next_contract(&rig.tw);
    let (quote, queued) = command(rig, captain, &format!("+passage/offer #{}={fare}", npc.0));
    assert!(
        queued.is_empty(),
        "quoting has no authoritative effects: {quote}"
    );
    assert_eq!(next_contract(&rig.tw), before, "a quote is not a contract");
    let (out, queued) = command(rig, captain, &acceptance(&quote));
    assert!(out.contains("Passage agreement filed"), "{out}");
    assert_eq!(queued.len(), 1, "one accepted booking: {out}");
    land_all(rig, &queued);
    assert_eq!(booking(rig, before)["phase"], "Booked");
    assert_eq!(
        survey_contract(&rig.tw, before),
        mush_economy::ContractState::Offered
    );
    pump(rig, 4);
    assert_eq!(
        survey_contract(&rig.tw, before),
        mush_economy::ContractState::Active
    );
    let row = booking(rig, before);
    assert_eq!(
        survey_money(&rig.tw, row["purse"].as_u64().unwrap()).1,
        row["fare_minor"].as_i64().unwrap()
    );
    (before, row)
}

fn invite(rig: &mut Rig, captain: ObjId, npc: ObjId, contract: u64) {
    let (out, queued) = command(rig, captain, &format!("+passage/board #{}", npc.0));
    assert_eq!(queued.len(), 1, "boarding queues its durable phase: {out}");
    land_all(rig, &queued);
    pump(rig, 2);
    assert_eq!(booking(rig, contract)["phase"], "Aboard", "{out}");
    assert!(matches!(
        parent(&rig.tw, npc),
        SpatialParent::Interior { .. }
    ));
}

fn captain_boards_at_stickney(rig: &mut Rig, captain: ObjId) {
    let gallery = station_room(&rig.tw, "stickney_port", "room.p01_berth_gallery");
    teleport(&mut rig.tw, captain, gallery);
    let out = run(&mut rig.tw, captain, "+ship/board");
    assert!(out.contains("You cycle the lock into"), "{out}");
}

fn conversation_status(rig: &mut Rig, npc: ObjId, asker: ObjId) -> String {
    let provider = rig
        .tw
        .engine()
        .registry
        .interface_provider("passenger")
        .unwrap();
    let reply = support::with_ctx(&mut rig.tw, |ctx| {
        ctx.actor = asker;
        provider.invoke(
            ctx,
            mush_core::AbilityCall { npc, asker },
            "passenger.status",
            &json!({}),
        )
    })
    .expect("present player can ask about the traveller's status");
    assert_eq!(reply.audience, mush_core::ReplyAudience::Asker);
    reply.text
}

#[test]
fn passenger_negotiation_requires_exact_unexpired_captain_acceptance() {
    let (mut rig, captain, _row, npc) = at_stickney();
    let contract = next_contract(&rig.tw);
    let provider = rig
        .tw
        .engine()
        .registry
        .interface_provider("passenger")
        .expect("passenger conversation service");
    let reply = support::with_ctx(&mut rig.tw, |ctx| {
        ctx.actor = captain;
        provider.invoke(
            ctx,
            mush_core::AbilityCall {
                npc,
                asker: captain,
            },
            "passenger.offer",
            &json!({"fare_minor": 99_900}),
        )
    })
    .expect("the NPC can quote the current captain's proposed fare");
    assert_eq!(reply.audience, mush_core::ReplyAudience::Room);
    assert!(acceptance(&reply.text).contains("=50000."));
    let refused = support::with_ctx(&mut rig.tw, |ctx| {
        ctx.actor = captain;
        provider.invoke(
            ctx,
            mush_core::AbilityCall {
                npc,
                asker: captain,
            },
            "passenger.offer",
            &json!({"fare_minor": 30_000, "actor": GOD}),
        )
    });
    assert!(
        refused.is_err(),
        "the model cannot override the actual asker"
    );
    assert_eq!(
        next_contract(&rig.tw),
        contract,
        "the conversational offer spends nothing"
    );
    let (quote, queued) = command(
        &mut rig,
        captain,
        &format!("+passage/offer #{}=999.00", npc.0),
    );
    assert!(quote.contains("That's beyond me"), "{quote}");
    assert!(
        acceptance(&quote).contains("=50000."),
        "bounded counteroffer: {quote}"
    );
    assert!(queued.is_empty());
    assert_eq!(next_contract(&rig.tw), contract);
    let forged = acceptance(&quote).replace("=50000.", "=1.");
    let (out, queued) = command(&mut rig, captain, &forged);
    assert!(out.contains("stale") || out.contains("changed"), "{out}");
    assert!(queued.is_empty());
    assert_eq!(next_contract(&rig.tw), contract);
    let now = rig.tw.world().now();
    rig.tw.set_now(now + 601);
    let (out, queued) = command(&mut rig, captain, &acceptance(&quote));
    assert!(out.contains("stale"), "{out}");
    assert!(queued.is_empty());
    assert_eq!(next_contract(&rig.tw), contract);
    let (contract, booked) = book(&mut rig, captain, npc, "350.00");
    assert_eq!(booked["fare_minor"], 35_000);
    let other = pilot(&mut rig, "OtherCaptain", 202);
    let bar = rig.tw.world().get(npc).unwrap().location.unwrap();
    teleport(&mut rig.tw, other, bar);
    let (out, queued) = command(&mut rig, other, &format!("+passage/offer #{}=400", npc.0));
    assert!(out.contains("already") || out.contains("second"), "{out}");
    assert!(queued.is_empty());
    assert_eq!(booking(&mut rig, contract), booked);
    no_refused_writes(&rig.tw);
}

#[test]
fn passenger_real_voyage_survives_restart_and_pays_and_remembers_once() {
    let (mut rig, captain, hull, npc) = at_stickney();
    let (contract, booked) = book(&mut rig, captain, npc, "350.00");
    let purse = booked["purse"].as_u64().unwrap();
    let payee = booked["payee"].as_u64().unwrap();
    let before_purse = survey_money(&rig.tw, purse);
    let before_payee = survey_money(&rig.tw, payee);
    invite(&mut rig, captain, npc, contract);
    captain_boards_at_stickney(&mut rig, captain);
    let interior = parent(&rig.tw, npc);
    assert_eq!(
        parent(&rig.tw, captain),
        interior,
        "both souls occupy the real berth space"
    );
    rig = reboot(rig, &[captain]);
    pump(&mut rig, 2);
    assert_eq!(parent(&rig.tw, npc), interior);
    assert_eq!(booking(&mut rig, contract)["phase"], "Aboard");
    assert_eq!(
        survey_money(&rig.tw, payee),
        before_payee,
        "boarding and reboot do not pay"
    );

    let ticket = file_and_launch(&mut rig, captain, ObjId(hull.hull), "+ship/undock ceres");
    assert_eq!(
        ticket.manifest.occupants, 2,
        "the NPC is a real manifest occupant"
    );
    arrive(&mut rig, &ticket, "ceres_port");
    for _ in 0..8 {
        survey_sweep(&mut rig);
        if matches!(
            survey_contract(&rig.tw, contract),
            mush_economy::ContractState::Settled { .. }
        ) {
            break;
        }
    }
    pump(&mut rig, 3);
    assert_eq!(booking(&mut rig, contract)["phase"], "Delivered");
    assert!(matches!(
        survey_contract(&rig.tw, contract),
        mush_economy::ContractState::Settled { .. }
    ));
    assert_eq!(survey_money(&rig.tw, purse), (before_purse.0 - 35_000, 0));
    assert_eq!(survey_money(&rig.tw, payee).0, before_payee.0 + 35_000);
    assert!(
        !matches!(parent(&rig.tw, npc), SpatialParent::Interior { .. }),
        "passenger leaves the ship"
    );
    let visitor = rig.tw.spawn_player("ArrivalVisitor");
    let room = rig.tw.world().get(npc).unwrap().location.unwrap();
    teleport(&mut rig.tw, visitor, room);
    let (status, queued) = command(&mut rig, visitor, "+passage Leila");
    assert!(queued.is_empty());
    let reply = conversation_status(&mut rig, npc, visitor);
    for text in [&status, &reply] {
        assert!(text.contains("reached my destination"), "{text}");
        assert!(
            !text.contains("Passenger Moth")
                && !text.contains("agreed fare")
                && !text.contains("Captain"),
            "private agreement is absent: {text}"
        );
    }
    let (status, queued) = command(&mut rig, captain, "+passage Leila");
    assert!(queued.is_empty());
    assert!(
        status.contains("arrived and paid")
            && status.contains("Passenger Moth")
            && status.contains("agreed fare"),
        "captain keeps agreement details: {status}"
    );
    let host = rig
        .tw
        .world()
        .module_state::<mush_npc::NpcHostState>()
        .unwrap();
    let memories = host
        .personal_memory
        .record_for(npc)
        .expect("actual journey memory")
        .clone();
    let outcomes: Vec<_> = memories
        .experiences
        .iter()
        .filter(|event| {
            event.kind == mush_npc::ExperienceKind::Outcome && event.speaker == Some(captain)
        })
        .collect();
    assert_eq!(outcomes.len(), 1);
    assert!(outcomes[0].text.to_lowercase().contains("ceres"));
    assert_eq!(host.relationships.personal(npc, captain).trust, 10);
    assert_eq!(host.relationships.personal(npc, captain).disposition, 5);
    let paid = survey_money(&rig.tw, payee);
    let renamed = run(
        &mut rig.tw,
        GOD,
        &format!("@name #{}=Captain Renamed", captain.0),
    );
    assert_eq!(
        rig.tw.world().get(captain).unwrap().name.as_str(),
        "Captain Renamed",
        "{renamed}"
    );
    rig = reboot(rig, &[captain]);
    let capture = Capture::default();
    tracing::dispatcher::with_default(&capture_dispatch(capture.clone()), || {
        pump(&mut rig, 3);
        survey_sweep(&mut rig);
    });
    assert_eq!(
        capture.count("passenger memory awaiting reconciliation"),
        0,
        "immutable completion text survives a captain rename: {}",
        capture.text()
    );
    assert_eq!(survey_money(&rig.tw, payee), paid);
    let host = rig
        .tw
        .world()
        .module_state::<mush_npc::NpcHostState>()
        .unwrap();
    assert_eq!(host.personal_memory.record_for(npc), Some(&memories));
    assert_eq!(host.relationships.personal(npc, captain).trust, 10);
    no_refused_writes(&rig.tw);
}

#[test]
fn passenger_predeparture_cancellation_refunds_without_work_credit() {
    let (mut rig, captain, _hull, npc) = at_stickney();
    let (contract, booked) = book(&mut rig, captain, npc, "300");
    let purse = booked["purse"].as_u64().unwrap();
    let payee = booked["payee"].as_u64().unwrap();
    let before = (survey_money(&rig.tw, purse), survey_money(&rig.tw, payee));
    let (out, queued) = command(&mut rig, captain, &format!("+passage/cancel #{}", npc.0));
    assert_eq!(
        queued.len(),
        1,
        "cancellation is a committed receipt: {out}"
    );
    land_all(&mut rig, &queued);
    for _ in 0..8 {
        survey_sweep(&mut rig);
    }
    pump(&mut rig, 2);
    assert_eq!(booking(&mut rig, contract)["phase"], "Cancelled");
    assert_eq!(survey_money(&rig.tw, purse), (before.0.0, 0));
    assert_eq!(survey_money(&rig.tw, payee), before.1);
    let host = rig
        .tw
        .world()
        .module_state::<mush_npc::NpcHostState>()
        .unwrap();
    assert_eq!(host.relationships.personal(npc, captain).trust, 0);
    assert_eq!(host.personal_memory.experience_count(npc), 0);
    rig = reboot(rig, &[captain]);
    pump(&mut rig, 2);
    assert_eq!(
        survey_money(&rig.tw, purse),
        (before.0.0, 0),
        "no fresh purse after restart"
    );
}

#[test]
fn passenger_boarding_obeys_real_berth_capacity_and_holds_departure_until_aboard() {
    let (mut rig, captain, hull, npc) = at_stickney();
    let (contract, booked) = book(&mut rig, captain, npc, "300");
    captain_boards_at_stickney(&mut rig, captain);
    let SpatialParent::Interior { space, slot } = parent(&rig.tw, captain) else {
        panic!("captain aboard")
    };
    assert!(slot.is_some(), "the ship uses its actual berth compartment");
    let extra = rig
        .tw
        .world_mut()
        .create(ObjKind::Npc, "Existing guest", GOD, Some(space))
        .unwrap();
    support::with_ctx(&mut rig.tw, |ctx| {
        mush_sim::place_interior(ctx, extra, space, slot)
    })
    .expect("second berth occupied");
    let (out, queued) = command(&mut rig, captain, &format!("+passage/board #{}", npc.0));
    assert!(
        queued.is_empty(),
        "full berths cannot book an aboard phase: {out}"
    );
    assert_eq!(booking(&mut rig, contract)["phase"], "Booked");
    assert!(!matches!(
        parent(&rig.tw, npc),
        SpatialParent::Interior { .. }
    ));
    assert_eq!(
        survey_money(&rig.tw, booked["purse"].as_u64().unwrap()).1,
        30_000
    );
    support::with_ctx(&mut rig.tw, |ctx| {
        mush_sim::unplace(ctx, extra, "fixture guest leaves")
    })
    .unwrap();
    let gallery = station_room(&rig.tw, "stickney_port", "room.p01_berth_gallery");
    teleport(&mut rig.tw, extra, gallery);

    // A funded agreement cannot be abandoned by filing departure with its
    // passenger still ashore, even after a berth becomes available.
    let (out, burn) = command(&mut rig, captain, "+ship/undock ceres");
    assert!(
        out.contains("Board your booked passenger or cancel"),
        "{out}"
    );
    assert!(
        burn.is_empty(),
        "no departure burn before the passenger boards"
    );
    assert_eq!(booking(&mut rig, contract), booked);
    assert!(!matches!(
        parent(&rig.tw, npc),
        SpatialParent::Interior { .. }
    ));
    assert_eq!(
        survey_money(&rig.tw, booked["purse"].as_u64().unwrap()).1,
        30_000
    );
    invite(&mut rig, captain, npc, contract);
    assert_eq!(booking(&mut rig, contract)["phase"], "Aboard");
    let ticket = file_and_launch(&mut rig, captain, ObjId(hull.hull), "+ship/undock ceres");
    assert_eq!(ticket.manifest.occupants, 2);
}

#[test]
fn passenger_held_quote_cannot_book_after_departure_clearance_is_filed() {
    let (mut rig, captain, hull, npc) = at_stickney();
    let contract = next_contract(&rig.tw);
    let (quote, quoted) = command(&mut rig, captain, &format!("+passage/offer #{}=300", npc.0));
    assert!(quoted.is_empty());
    let acceptance = acceptance(&quote);
    captain_boards_at_stickney(&mut rig, captain);
    let (out, burn) = command(&mut rig, captain, "+ship/undock ceres");
    assert!(out.contains("Clearance filed for"), "{out}");
    assert_eq!(burn.len(), 1);
    let before = rig
        .tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .unwrap()
        .snapshot()
        .clone();
    let (out, accepted) = command(&mut rig, captain, &acceptance);
    assert!(
        accepted.is_empty(),
        "a quote cannot change the filed manifest: {out}"
    );
    assert!(!out.contains("Passage agreement filed"), "{out}");
    assert_eq!(next_contract(&rig.tw), contract);
    assert_eq!(
        rig.tw
            .world()
            .module_state::<mush_economy::EconomyStore>()
            .unwrap()
            .snapshot(),
        &before
    );
    assert!(!matches!(
        parent(&rig.tw, npc),
        SpatialParent::Interior { .. }
    ));
    assert!(tacsim::port::voyage::pending_of(rig.tw.world(), ObjId(hull.hull)).is_some());
    no_refused_writes(&rig.tw);
}

#[test]
fn passenger_wrong_destination_keeps_escrow_and_grants_no_completion() {
    let (mut rig, captain, hull, npc) = at_stickney();
    // Exercise the reusable role with a different authored destination. The
    // real Stickney->Ceres passage below is only an intermediate stop for it.
    let mut attr = rig
        .tw
        .world()
        .attr(npc, tacsim::port::passenger::TRAVELLER_ATTR)
        .unwrap()
        .clone();
    let mut def = attr.as_json().unwrap().clone();
    def["destination"] = json!("luna");
    attr.value = mush_core::Attr::json(def, GOD).value;
    rig.tw
        .world_mut()
        .set_attr(npc, tacsim::port::passenger::TRAVELLER_ATTR, attr)
        .unwrap();
    let (contract, booked) = book(&mut rig, captain, npc, "300");
    let purse = booked["purse"].as_u64().unwrap();
    let payee = booked["payee"].as_u64().unwrap();
    let before = (survey_money(&rig.tw, purse), survey_money(&rig.tw, payee));
    invite(&mut rig, captain, npc, contract);
    captain_boards_at_stickney(&mut rig, captain);
    let ticket = file_and_launch(&mut rig, captain, ObjId(hull.hull), "+ship/undock ceres");
    arrive(&mut rig, &ticket, "ceres_port");
    survey_sweep(&mut rig);
    pump(&mut rig, 2);
    assert_eq!(booking(&mut rig, contract)["phase"], "Aboard");
    assert_eq!(
        survey_contract(&rig.tw, contract),
        mush_economy::ContractState::Active
    );
    assert_eq!(survey_money(&rig.tw, purse), before.0);
    assert_eq!(survey_money(&rig.tw, payee), before.1);
    assert!(matches!(
        parent(&rig.tw, npc),
        SpatialParent::Interior { .. }
    ));
    let host = rig
        .tw
        .world()
        .module_state::<mush_npc::NpcHostState>()
        .unwrap();
    assert_eq!(host.relationships.personal(npc, captain).trust, 0);
    assert_eq!(host.personal_memory.experience_count(npc), 0);
    no_refused_writes(&rig.tw);
}

#[test]
fn passenger_boarding_receipt_blocks_departure_and_recovers_after_restart() {
    let (mut rig, captain, hull, npc) = at_stickney();
    let (contract, _) = book(&mut rig, captain, npc, "300");
    let (out, boarding) = command(&mut rig, captain, &format!("+passage/board #{}", npc.0));
    assert_eq!(boarding.len(), 1, "{out}");
    assert_eq!(booking(&mut rig, contract)["phase"], "Booked");
    assert!(matches!(
        parent(&rig.tw, npc),
        SpatialParent::Interior { .. }
    ));
    captain_boards_at_stickney(&mut rig, captain);
    let (out, departure) = command(&mut rig, captain, "+ship/undock ceres");
    assert!(out.contains("still being confirmed"), "{out}");
    assert!(
        departure.is_empty(),
        "no burn before the manifest commits: {out}"
    );

    // Lose the in-flight receipt while retaining the real berth placement.
    rig = reboot(rig, &[captain]);
    let (out, departure) = command(&mut rig, captain, "+ship/undock ceres");
    assert!(
        departure.is_empty(),
        "unhydrated or unsettled manifest holds: {out}"
    );
    pump(&mut rig, 4);
    assert_eq!(booking(&mut rig, contract)["phase"], "Aboard");
    let ticket = file_and_launch(&mut rig, captain, ObjId(hull.hull), "+ship/undock ceres");
    assert_eq!(ticket.manifest.occupants, 2);
    no_refused_writes(&rig.tw);
}

#[test]
fn passenger_cancellation_receipt_blocks_departure_and_recovers_after_restart() {
    let (mut rig, captain, _hull, npc) = at_stickney();
    let (contract, booked) = book(&mut rig, captain, npc, "300");
    let purse = booked["purse"].as_u64().unwrap();
    let payee = booked["payee"].as_u64().unwrap();
    let before = (survey_money(&rig.tw, purse), survey_money(&rig.tw, payee));
    invite(&mut rig, captain, npc, contract);
    captain_boards_at_stickney(&mut rig, captain);
    let (out, cancellation) = command(&mut rig, captain, &format!("+passage/cancel #{}", npc.0));
    assert_eq!(cancellation.len(), 1, "{out}");
    assert_eq!(booking(&mut rig, contract)["phase"], "Aboard");
    assert!(!matches!(
        parent(&rig.tw, npc),
        SpatialParent::Interior { .. }
    ));
    let (out, departure) = command(&mut rig, captain, "+ship/undock ceres");
    assert!(out.contains("still being confirmed"), "{out}");
    assert!(
        departure.is_empty(),
        "no burn before cancellation commits: {out}"
    );

    rig = reboot(rig, &[captain]);
    pump(&mut rig, 4);
    for _ in 0..8 {
        survey_sweep(&mut rig);
    }
    assert_eq!(booking(&mut rig, contract)["phase"], "Cancelled");
    assert_eq!(survey_money(&rig.tw, purse), (before.0.0, 0));
    assert_eq!(survey_money(&rig.tw, payee), before.1);
    let (out, departure) = command(&mut rig, captain, "+ship/undock ceres");
    assert!(
        out.contains("Clearance filed for"),
        "the recovered cancellation releases departure: {out}"
    );
    assert_eq!(departure.len(), 1);
    no_refused_writes(&rig.tw);
}

#[test]
fn passenger_expired_untravelled_booking_refunds_and_releases_the_role() {
    let (mut rig, captain, _hull, npc) = at_stickney();
    let (contract, booked) = book(&mut rig, captain, npc, "300");
    let purse = booked["purse"].as_u64().unwrap();
    let payee = booked["payee"].as_u64().unwrap();
    let before = (survey_money(&rig.tw, purse), survey_money(&rig.tw, payee));
    let terminal = booked["deadline_ic"].as_i64().unwrap() + 86_400 + 61;
    let _ = to_ic(&mut rig, terminal);
    for _ in 0..8 {
        survey_sweep(&mut rig);
        pump(&mut rig, 2);
    }
    assert_eq!(booking(&mut rig, contract)["phase"], "Cancelled");
    assert_eq!(survey_money(&rig.tw, purse), (before.0.0, 0));
    // Other account activity during a full IC year is unrelated; the passenger
    // contract must not contain a payment to this captain.
    assert!(!matches!(
        survey_contract(&rig.tw, contract),
        mush_economy::ContractState::Settled { .. }
    ));
    let host = rig
        .tw
        .world()
        .module_state::<mush_npc::NpcHostState>()
        .unwrap();
    assert_eq!(host.relationships.personal(npc, captain).trust, 0);
    assert_eq!(host.personal_memory.experience_count(npc), 0);
    rig = reboot(rig, &[captain]);
    pump(&mut rig, 2);
    assert_eq!(booking(&mut rig, contract)["phase"], "Cancelled");
    assert_eq!(survey_money(&rig.tw, purse), (before.0.0, 0));

    // A fresh captain avoids coupling role reuse to an old charter's annual
    // loan/recall consequences. This is another real Ceres->Stickney journey.
    let (next, hull) = charter(&mut rig, "NextCaptain", 203, "Second Passage");
    load(&mut rig, &hull, 150, 200, 91_000);
    board(&mut rig, next);
    let ticket = file_and_launch(&mut rig, next, ObjId(hull.hull), "+ship/undock stickney");
    arrive(&mut rig, &ticket, "stickney_port");
    let out = run(&mut rig.tw, next, "+ship/leave");
    assert!(out.contains("You cycle the lock out into"), "{out}");
    let room = rig.tw.world().get(npc).unwrap().location.unwrap();
    teleport(&mut rig.tw, next, room);
    let (status, queued) = command(&mut rig, next, "+passage Leila");
    assert!(queued.is_empty());
    let reply = conversation_status(&mut rig, npc, next);
    for text in [&status, &reply] {
        assert!(
            text.contains("looking for passage to ceres") && text.contains("+passage/offer"),
            "released cancellation is available again: {text}"
        );
        assert!(
            !text.contains("Passenger Moth")
                && !text.contains("agreed fare")
                && !text.contains("Captain"),
            "old agreement stays private: {text}"
        );
    }
    let (status, queued) = command(&mut rig, captain, "+passage Leila");
    assert!(queued.is_empty());
    assert!(
        status.contains("cancelled")
            && status.contains("Passenger Moth")
            && status.contains("agreed fare"),
        "old captain keeps agreement details: {status}"
    );
    let (new_contract, new_booking) = book(&mut rig, next, npc, "300");
    assert_ne!(new_contract, contract);
    assert_eq!(new_booking["captain"], json!(next));
    assert_eq!(survey_money(&rig.tw, purse), (before.0.0, 30_000));
    no_refused_writes(&rig.tw);
}

#[test]
fn passenger_boarding_fails_closed_when_the_saved_clearance_is_unreadable() {
    let (mut rig, captain, hull, npc) = at_stickney();
    let (contract, booked) = book(&mut rig, captain, npc, "300");
    let key = tacsim::port::voyage::voyage_key(ObjId(hull.hull));
    rig = reboot_editing(rig, key.as_str(), &[captain], |doc| {
        doc["schema"] = json!(999);
    });
    pump(&mut rig, 2);
    let before = parent(&rig.tw, npc);
    let (out, queued) = command(&mut rig, captain, &format!("+passage/board #{}", npc.0));
    assert!(
        queued.is_empty(),
        "an unreadable clearance cannot be treated as absent: {out}"
    );
    assert_eq!(parent(&rig.tw, npc), before);
    assert_eq!(booking(&mut rig, contract), booked);
    assert_eq!(
        survey_money(&rig.tw, booked["purse"].as_u64().unwrap()).1,
        30_000
    );
    no_refused_writes(&rig.tw);
}

#[test]
fn passenger_conversation_quote_cannot_override_a_newly_committed_booking() {
    let (mut rig, captain, _hull, npc) = at_stickney();
    let contract = next_contract(&rig.tw);
    let (quote, _) = command(&mut rig, captain, &format!("+passage/offer #{}=300", npc.0));
    let (out, queued) = command(&mut rig, captain, &acceptance(&quote));
    assert_eq!(queued.len(), 1, "{out}");
    land_all(&mut rig, &queued);
    let booked = booking(&mut rig, contract);
    assert_eq!(booked["phase"], "Booked");

    // No port tick has refreshed the conversation projection since the typed
    // booking landed. A read-only NPC quote may therefore be out of date.
    let provider = rig
        .tw
        .engine()
        .registry
        .interface_provider("passenger")
        .unwrap();
    let reply = support::with_ctx(&mut rig.tw, |ctx| {
        ctx.actor = captain;
        provider.invoke(
            ctx,
            mush_core::AbilityCall {
                npc,
                asker: captain,
            },
            "passenger.offer",
            &json!({"fare_minor": 35_000}),
        )
    })
    .expect("the stale read-only projection can only quote, never book");
    let next = next_contract(&rig.tw);
    let (out, queued) = command(&mut rig, captain, &acceptance(&reply.text));
    assert!(
        out.contains("already") || out.contains("second"),
        "live acceptance rejects stale conversation: {out}"
    );
    assert!(queued.is_empty());
    assert_eq!(next_contract(&rig.tw), next);
    assert_eq!(booking(&mut rig, contract), booked);
    no_refused_writes(&rig.tw);
}

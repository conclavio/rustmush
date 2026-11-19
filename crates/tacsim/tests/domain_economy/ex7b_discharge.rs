//! **Doc 48 cut EX.7b — a player discharges hired crew at a port**, over the composed lane
//! (`driving`'s rig) at the live rate, 24:1, where an IC day is one real hour.
//!
//! 1. **A hire discharged:** a captain's rating runs three IC days and is discharged at Ceres.
//!    The final wage, the day in progress, moves once to the hiring port; the wage is ended, the
//!    contract and the owed exit land, the rating is on offer again and another captain hires
//!    them; no later fire charges the first captain anything.
//! 2. **A broke employer:** five missed days and the day in progress stand as one 60 cr debt; no
//!    further day accrues; funded, she pays it at the next port fire, once.
//! 3. **The refusals:** a stranger, an unhired NPC, a malformed name, an employer away from every
//!    port, an employer with a request in flight, and a hire in custody.
//! 4. **A hire aboard, and a hire in a seat:** a boarded rating leaves the ship, and one seated in
//!    a rig standing in the carrier's bay leaves the seat; each is back in the pool room. Neither
//!    is discharged while the ship or the rig is under a field's mark.
//! 5. **A hire out with a unit:** seated in a rig launched out of the bay, a hire is not
//!    discharged.
//!
//! **What is a fixture.** The captains' money (minted by staff); the seat (a staff hand, the
//! billet a DT flight's activation writes, CR.1's precedent); the custody record (the crew
//! book's test seam); a field's mark (a staff hand, as a field's admission writes it); the
//! captain set down at a port while her rig is out. The hires, the
//! discharges, the wages and the roster's half are the production command paths.

use mush_core::ObjId;
use mush_org::PrincipalId;
use tacsim::detachment::crew::{discharge as game, wages};
use tacsim::field::npc_exit_of;

use crate::driving_rig::{
    Rig, command, drain, land_composed, run, set_rate, staff_hand, stocked, sweep, teleport,
};
use crate::ex7_wages::{
    MENDES, OKONKWO, authority, berth_paid, broke, day, employer, hire, npc, wage,
};
use crate::hull_resale_ex2::{account_of, balance, captain, mint};

/// Where `who` stands.
fn room_of(rig: &Rig, who: ObjId) -> ObjId {
    rig.tw
        .world()
        .get(who)
        .and_then(|o| o.location)
        .expect("a location")
}

/// **`who` discharges `npc`**: what they were told, the roster's half landed.
fn discharge(rig: &mut Rig, who: ObjId, npc: ObjId) -> String {
    // What the ticks queued for other actors meanwhile is not this story's.
    let _ = drain(&mut rig.tw);
    let (said, queued) = command(rig, who, &format!("+sortie/discharge #{}", npc.0));
    assert_eq!(queued.len(), 1, "one roster act: {said}");
    land_composed(rig, &queued[0]).expect("the roster's half lands");
    said
}

/// Whether `npc` is on offer to `who` where `who` stands.
fn on_offer(rig: &mut Rig, who: ObjId, npc: ObjId) -> bool {
    run(&mut rig.tw, who, "+sortie/crew").contains(&format!("+sortie/hire #{}=", npc.0))
}

/// A sentence's fixed head, up to its first placeholder after `{name}`.
fn tail(sentence: &'static str) -> &'static str {
    sentence.strip_prefix("{name}").unwrap_or(sentence)
}

/// **A hire discharged**: the final wage moves once, the wage and the contract end, the rating is
/// hired again by another captain, and nothing more is charged.
#[test]
#[allow(clippy::too_many_lines)] // one story, in order
fn ex7b_a_hire_is_discharged_at_port_and_its_final_wage_moves_once() {
    let mut rig = stocked();
    let who = employer(&mut rig, "Employer", 1, "Salt Moth");
    mint(&mut rig, who, 100_000);
    set_rate(&mut rig.tw, 24);
    let mendes = npc(&rig, MENDES);
    let pool = room_of(&rig, mendes);
    let _ = hire(&mut rig, who, mendes);
    sweep(&mut rig.tw, 60);
    assert!(wage(&rig, who, mendes).is_some(), "the wage is charged");
    let contract = run(&mut rig.tw, who, &format!("+sortie/contract #{}", mendes.0));
    assert!(
        contract.contains(&format!(
            "End the hire at a port: +sortie/discharge #{}",
            mendes.0
        )),
        "{contract}"
    );

    let hall = authority(&rig);
    let (mine, theirs) = (balance(&rig, account_of(&rig, who)), balance(&rig, hall));
    let berthed = berth_paid(&rig, who);
    for _ in 0..3 {
        day(&mut rig);
    }
    assert_eq!(
        wage(&rig, who, mendes).map(|o| o.settled_total),
        Some(3),
        "three IC days paid"
    );

    // The discharge: the day in progress, whole, to the same payee, at once.
    let _ = drain(&mut rig.tw);
    let (said, queued) = command(&mut rig, who, &format!("+sortie/discharge #{}", mendes.0));
    assert!(
        said.contains("Ilya Mendes is discharged: 10.00cr paid to ")
            && said.contains(" as the final wage."),
        "{said}"
    );
    assert!(
        said.contains("The contract is ended. Ilya Mendes is back at ")
            && said.contains(", on offer there again."),
        "{said}"
    );
    assert_eq!(queued.len(), 1, "one roster act: {said}");
    let berth = berth_paid(&rig, who) - berthed;
    assert_eq!(
        balance(&rig, account_of(&rig, who)),
        mine - 3 * 1_000 - 1_000 - berth,
        "three days and the final day, beside her berth"
    );
    assert_eq!(balance(&rig, hall), theirs + 4 * 1_000 + berth);
    assert!(wage(&rig, who, mendes).is_none(), "the wage is ended");
    assert_eq!(wages::arrears_of(rig.tw.world(), who, mendes), 0);
    assert_eq!(room_of(&rig, mendes), pool);

    // Until the roster's half lands the exit is owed: the contract reads ended, and a day's
    // fires neither open the wage again nor charge anything.
    assert!(npc_exit_of(rig.tw.world(), mendes).is_some());
    let ended = run(&mut rig.tw, who, &format!("+sortie/contract #{}", mendes.0));
    assert!(ended.contains("Crew or contract unavailable"), "{ended}");
    let again = run(
        &mut rig.tw,
        who,
        &format!("+sortie/discharge #{}", mendes.0),
    );
    assert!(again.contains(game::NOT_YOURS), "{again}");
    day(&mut rig);
    assert!(
        wage(&rig, who, mendes).is_none(),
        "no wage opens for a hire who left"
    );
    let berth = berth_paid(&rig, who) - berthed;
    assert_eq!(
        balance(&rig, account_of(&rig, who)),
        mine - 4 * 1_000 - berth,
        "nothing more is charged"
    );

    // The roster's half: the contract is gone, the exit forgotten, the rating on offer again.
    land_composed(&mut rig, &queued[0]).expect("the roster's half lands");
    let _ = drain(&mut rig.tw);
    sweep(&mut rig.tw, 60);
    let _ = drain(&mut rig.tw);
    assert!(
        npc_exit_of(rig.tw.world(), mendes).is_none(),
        "the owed exit is forgotten"
    );
    let ended = run(&mut rig.tw, who, &format!("+sortie/contract #{}", mendes.0));
    assert!(ended.contains("Crew or contract unavailable"), "{ended}");
    teleport(&mut rig.tw, who, pool);
    assert!(
        on_offer(&mut rig, who, mendes),
        "the rating is on offer again"
    );

    // Another captain hires the rating; the first owes nothing more over two IC days.
    let second = employer(&mut rig, "Second", 2, "Red Moth");
    mint(&mut rig, second, 100_000);
    let _ = hire(&mut rig, second, mendes);
    sweep(&mut rig.tw, 60);
    assert!(
        wage(&rig, second, mendes).is_some(),
        "the second hire's wage is charged"
    );
    assert!(wage(&rig, who, mendes).is_none());
    for _ in 0..2 {
        day(&mut rig);
    }
    let berth = berth_paid(&rig, who) - berthed;
    assert_eq!(
        balance(&rig, account_of(&rig, who)),
        mine - 4 * 1_000 - berth,
        "the discharged hire costs its old employer nothing more"
    );
    assert_eq!(
        wage(&rig, second, mendes).map(|o| o.settled_total),
        Some(2),
        "the new employer pays"
    );
}

/// **A broke employer**: the arrears and the day in progress stand as her debt, no further day
/// accrues, and she pays it once, at the first port fire that finds her funded.
#[test]
fn ex7b_a_broke_employers_discharge_leaves_its_arrears_paid_once_funded() {
    let mut rig = stocked();
    let who = employer(&mut rig, "Broke", 2, "Red Moth");
    broke(&mut rig, who);
    set_rate(&mut rig.tw, 24);
    let mendes = npc(&rig, MENDES);
    let _ = hire(&mut rig, who, mendes);
    sweep(&mut rig.tw, 60);
    for _ in 0..40 {
        if wage(&rig, who, mendes).is_some_and(|o| o.missed_total >= 5) {
            break;
        }
        day(&mut rig);
    }
    assert_eq!(wages::arrears_of(rig.tw.world(), who, mendes), 5_000);

    // Never refused for want of money: 50 cr of arrears and the 10 cr day stand as her debt.
    let said = discharge(&mut rig, who, mendes);
    assert!(
        said.contains(
            "Ilya Mendes is discharged: the final wage, 60.00cr, stands as your debt to "
        ),
        "{said}"
    );
    assert!(wage(&rig, who, mendes).is_none(), "the wage is ended");
    assert_eq!(wages::arrears_of(rig.tw.world(), who, mendes), 6_000);
    assert_eq!(balance(&rig, account_of(&rig, who)), 0);
    let ended = run(&mut rig.tw, who, &format!("+sortie/contract #{}", mendes.0));
    assert!(
        ended.contains(&format!(
            "No crew contract with #{} stands; 60.00cr of its wage is your debt, collected when your account covers it.",
            mendes.0
        )),
        "{ended}"
    );

    // No further day accrues, however many fires follow.
    for _ in 0..3 {
        day(&mut rig);
        let _ = drain(&mut rig.tw);
    }
    assert_eq!(
        wages::arrears_of(rig.tw.world(), who, mendes),
        6_000,
        "the debt does not grow"
    );
    assert!(wage(&rig, who, mendes).is_none());

    // Funded with exactly what she owes, she pays the 60 cr at the next port fire, and only once.
    let hall = authority(&rig);
    mint(&mut rig, who, 6_000);
    let (theirs, berthed) = (balance(&rig, hall), berth_paid(&rig, who));
    sweep(&mut rig.tw, 60);
    assert_eq!(
        wages::arrears_of(rig.tw.world(), who, mendes),
        0,
        "the debt is paid"
    );
    assert_eq!(
        balance(&rig, account_of(&rig, who)),
        0,
        "to the last credit"
    );
    mint(&mut rig, who, 94_000);
    for _ in 0..2 {
        day(&mut rig);
    }
    let berth = berth_paid(&rig, who) - berthed;
    assert_eq!(
        balance(&rig, account_of(&rig, who)),
        100_000 - 6_000 - berth,
        "60 cr, once, beside her berth"
    );
    assert_eq!(balance(&rig, hall), theirs + 6_000 + berth);
    let ended = run(&mut rig.tw, who, &format!("+sortie/contract #{}", mendes.0));
    assert!(ended.contains("Crew or contract unavailable"), "{ended}");
}

/// A broke captain whose rating has missed five IC days: 50 cr in arrears under a wage in grace.
fn five_days_behind(rig: &mut Rig, name: &str) -> (ObjId, ObjId) {
    let who = employer(rig, name, 2, "Red Moth");
    broke(rig, who);
    set_rate(&mut rig.tw, 24);
    let mendes = npc(rig, MENDES);
    let _ = hire(rig, who, mendes);
    sweep(&mut rig.tw, 60);
    for _ in 0..40 {
        if wage(rig, who, mendes).is_some_and(|o| o.missed_total >= 5) {
            break;
        }
        day(rig);
    }
    assert_eq!(wages::arrears_of(rig.tw.world(), who, mendes), 5_000);
    (who, mendes)
}

/// **A funded employer's arrears are paid with the final day**: a standing wage's arrears are
/// the economy's to collect, so a port fire leaves them; the discharge pays them and the day.
#[test]
fn ex7b_a_funded_employers_arrears_are_paid_with_the_final_day() {
    let mut rig = stocked();
    let (who, mendes) = five_days_behind(&mut rig, "Behind");
    mint(&mut rig, who, 100_000);
    sweep(&mut rig.tw, 60);
    assert_eq!(
        wages::arrears_of(rig.tw.world(), who, mendes),
        5_000,
        "a standing wage's arrears wait for its next period, as EX.7 built them"
    );
    let (hall, mine) = (authority(&rig), balance(&rig, account_of(&rig, who)));
    let theirs = balance(&rig, hall);
    let said = discharge(&mut rig, who, mendes);
    assert!(
        said.contains("Ilya Mendes is discharged: 60.00cr paid to ")
            && said.contains(" as the final wage."),
        "{said}"
    );
    assert_eq!(wages::arrears_of(rig.tw.world(), who, mendes), 0);
    assert_eq!(balance(&rig, account_of(&rig, who)), mine - 6_000);
    assert_eq!(balance(&rig, hall), theirs + 6_000);
    assert!(wage(&rig, who, mendes).is_none(), "the wage is ended");
}

/// **A part-funded employer** pays what her account covers, a whole debt at a time: the day in
/// progress, and the arrears stand.
#[test]
fn ex7b_a_part_funded_employer_pays_the_day_and_owes_the_arrears() {
    let mut rig = stocked();
    let (who, mendes) = five_days_behind(&mut rig, "Short");
    mint(&mut rig, who, 1_000);
    let said = discharge(&mut rig, who, mendes);
    assert!(
        said.contains(
            "Ilya Mendes is discharged: 10.00cr of the final wage is paid and 50.00cr stands as your debt."
        ),
        "{said}"
    );
    assert_eq!(wages::arrears_of(rig.tw.world(), who, mendes), 5_000);
    assert_eq!(
        balance(&rig, account_of(&rig, who)),
        0,
        "exactly the day, to the last credit"
    );
    assert!(wage(&rig, who, mendes).is_none(), "the wage is ended");
}

/// **The refusals**, each with its own sentence, and nothing changed by any of them.
#[test]
#[allow(clippy::too_many_lines)] // one story, in order
fn ex7b_a_discharge_is_refused_with_its_own_sentence() {
    let mut rig = stocked();
    let (who, row) = captain(&mut rig, "Employer", 1, "Salt Moth");
    mint(&mut rig, who, 100_000);
    set_rate(&mut rig.tw, 24);
    let (mendes, okonkwo) = (npc(&rig, MENDES), npc(&rig, OKONKWO));
    let pool = room_of(&rig, mendes);
    let _ = hire(&mut rig, who, mendes);
    sweep(&mut rig.tw, 60);
    let before = balance(&rig, account_of(&rig, who));
    let discharge_line = format!("+sortie/discharge #{}", mendes.0);

    // A malformed name.
    for bad in ["+sortie/discharge", "+sortie/discharge Mendes"] {
        let said = run(&mut rig.tw, who, bad);
        assert!(said.contains(game::DISCHARGE_USAGE), "{bad}: {said}");
    }
    // A stranger, and an NPC nobody hired.
    let stranger = rig.tw.spawn_player("Stranger");
    teleport(&mut rig.tw, stranger, pool);
    let said = run(&mut rig.tw, stranger, &discharge_line);
    assert!(said.contains(game::NOT_YOURS), "{said}");
    let said = run(
        &mut rig.tw,
        who,
        &format!("+sortie/discharge #{}", okonkwo.0),
    );
    assert!(said.contains(game::NOT_YOURS), "{said}");

    // Away from every port's station: aboard her own ship.
    let aboard = ObjId(*row.rooms.values().next().expect("the hull has rooms"));
    teleport(&mut rig.tw, who, aboard);
    let said = run(&mut rig.tw, who, &discharge_line);
    assert!(said.contains(game::AT_PORT), "{said}");

    // A request of her own in flight: the officer's hire, not yet landed.
    let room = room_of(&rig, okonkwo);
    teleport(&mut rig.tw, who, room);
    let offers = run(&mut rig.tw, who, "+sortie/crew");
    let accept = offers
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix("Accept: ")
                .filter(|line| line.contains(&format!("#{}=", okonkwo.0)))
        })
        .unwrap_or_else(|| panic!("the officer is on offer: {offers}"))
        .to_owned();
    let _ = run(&mut rig.tw, who, &accept);
    let said = run(&mut rig.tw, who, &discharge_line);
    assert!(said.contains(game::BUSY), "{said}");
    let queued = drain(&mut rig.tw);
    assert_eq!(queued.len(), 1, "the hire alone was queued");
    land_composed(&mut rig, &queued[0]).expect("the officer's hire lands");

    // A hire in custody.
    staff_hand(&mut rig, |ctx| {
        tacsim::field::crew::hold_for_tests(
            ctx.w,
            tacsim::field::CrewRecord {
                format: tacsim::field::crew::CREW_FORMAT,
                character: okonkwo,
                location: tacsim::field::CrewLocation::Captured {
                    carrier: ObjId(row.hull),
                    captor: stranger,
                    since_ic: 0,
                },
                condition: tacsim::field::CrewCondition::Fit,
                at_ic: 0,
                organizer: None,
                extraction: None,
                treatment: None,
                resolve_permille: None,
            },
        );
    });
    let said = run(
        &mut rig.tw,
        who,
        &format!("+sortie/discharge #{}", okonkwo.0),
    );
    assert!(
        said.contains(&format!("Sana Okonkwo{}", tail(game::CAPTIVE))),
        "{said}"
    );

    // Nothing changed: both contracts stand, the rating's wage runs, no money moved.
    for hired in [mendes, okonkwo] {
        let contract = run(&mut rig.tw, who, &format!("+sortie/contract #{}", hired.0));
        assert!(contract.contains("Crew contract"), "{contract}");
        assert!(npc_exit_of(rig.tw.world(), hired).is_none());
    }
    assert!(wage(&rig, who, mendes).is_some());
    assert_eq!(balance(&rig, account_of(&rig, who)), before);
    assert!(
        mush_economy::debts_of(rig.tw.world(), &PrincipalId::Object(who), &[]).is_empty(),
        "no debt was written"
    );
}

/// **A field's mark on `unit`, by a staff hand**, as a field's admission writes it.
fn under_a_field(rig: &mut Rig, unit: ObjId) {
    staff_hand(rig, |ctx| {
        mush_sim::deployed::mark(
            ctx.w,
            unit,
            mush_sim::deployed::DeployedMark {
                match_id: 0,
                arena: "ahuna_port".into(),
                field: Some(mush_sim::deployed::DeployedField {
                    site: "ahuna_port".into(),
                    ordinal: 1,
                }),
            },
        );
    });
}

/// The field behind `unit`: its mark lifted, by the same hand.
fn field_behind(rig: &mut Rig, unit: ObjId) {
    staff_hand(rig, |ctx| {
        let _ = mush_sim::deployed::clear(ctx.w, unit);
    });
}

/// **A hire aboard, and a hire in a seat**: a boarded rating leaves the ship and a rating seated
/// in a rig standing in the carrier's bay leaves the seat; each is back in the pool room. While
/// the ship, or the rig, is under a field's mark the hire is not discharged.
#[test]
#[allow(clippy::too_many_lines)] // one story, in order
fn ex7b_a_boarded_hire_leaves_the_ship_and_a_seated_hire_its_seat() {
    let mut rig = stocked();
    let (who, row) = crate::driving_rig::prospector(&mut rig, "Employer", 1, "Salt Moth");
    mint(&mut rig, who, 1_000_000);
    set_rate(&mut rig.tw, 24);
    let mendes = npc(&rig, MENDES);
    let pool = room_of(&rig, mendes);
    let line = format!("+sortie/discharge #{}", mendes.0);

    // Boarded by the production path.
    let _ = hire(&mut rig, who, mendes);
    let boarded = run(&mut rig.tw, who, &format!("+sortie/board #{}", mendes.0));
    assert!(boarded.contains("has come aboard"), "{boarded}");
    assert_ne!(room_of(&rig, mendes), pool, "the rating is aboard");

    // Her ship in an open field: not discharged.
    under_a_field(&mut rig, ObjId(row.hull));
    let said = run(&mut rig.tw, who, &line);
    assert!(
        said.contains(&format!("Ilya Mendes{}", tail(game::IN_FIELD))),
        "aboard a carrier in a field: {said}"
    );
    assert_ne!(room_of(&rig, mendes), pool, "still aboard");
    field_behind(&mut rig, ObjId(row.hull));

    // Discharged from the station. No port fire has opened the wage yet: the day in progress is
    // owed all the same.
    let said = discharge(&mut rig, who, mendes);
    assert!(
        said.contains("Ilya Mendes is discharged: 10.00cr paid to "),
        "{said}"
    );
    assert_eq!(room_of(&rig, mendes), pool, "back in the pool room: {said}");
    sweep(&mut rig.tw, 60);
    let _ = drain(&mut rig.tw);
    assert!(
        on_offer(&mut rig, who, mendes),
        "on offer again in the pool room"
    );

    // Hired again and seated in a leased rig standing in her carrier's bay.
    let _ = hire(&mut rig, who, mendes);
    crate::driving_rig::lease_one(&mut rig, who);
    let unit = crate::driving_rig::store(&rig.tw)
        .tv_leases_of(who)
        .map(|lease| lease.tv)
        .next()
        .expect("a leased rig");
    staff_hand(&mut rig, |ctx| {
        ctx.w.move_to(mendes, unit).expect("the rating climbs in");
    });
    crate::field_raiders_rig::seat_npc(&mut rig, unit, mendes);
    let seated = |rig: &Rig| {
        rig.tw
            .world()
            .module_state::<mush_sim::SpatialState>()
            .and_then(|state| state.assignments.get(&unit))
            .is_some_and(|host| host.seats.values().any(|seat| seat.actor == mendes))
    };
    assert!(seated(&rig), "the rating holds the seat");
    teleport(&mut rig.tw, who, pool);

    // The rig in an open field: not discharged, and the seat kept.
    under_a_field(&mut rig, unit);
    let said = run(&mut rig.tw, who, &line);
    assert!(
        said.contains(&format!("Ilya Mendes{}", tail(game::IN_FIELD))),
        "in a rig in a field: {said}"
    );
    assert!(seated(&rig), "the seat is kept");
    field_behind(&mut rig, unit);

    let said = discharge(&mut rig, who, mendes);
    assert!(said.contains("Ilya Mendes is discharged: "), "{said}");
    assert!(!seated(&rig), "the seat is vacated");
    assert_eq!(room_of(&rig, mendes), pool, "back in the pool room");
}

/// **A hire out with a unit**: seated in a rig launched out of the carrier's bay onto a site, a
/// hire is not discharged.
#[test]
fn ex7b_a_hire_out_with_a_unit_is_not_discharged() {
    use crate::grounds_rig::AHUNA_FLANK as AT;
    let mut rig = crate::driving_rig::stocked_tuned(crate::custody_rig::quick);
    let mendes = npc(&rig, MENDES);
    let pool = room_of(&rig, mendes);
    let (who, _, tvs) = crate::driving_rig::on_pad_with_at(
        &mut rig,
        ("Pilot", 1, "Salt Moth"),
        AT.site(),
        |rig, who| {
            let _ = hire(rig, who, mendes);
            crate::driving_rig::at_counter(rig, who);
            let leased = run(&mut rig.tw, who, "+tv/lease go");
            assert!(leased.contains("leases you"), "{leased}");
        },
    );
    let escort = tvs[0];
    crate::driving_rig::launched_at(&mut rig, AT.entry, who, escort);
    staff_hand(&mut rig, |ctx| {
        ctx.w.move_to(mendes, escort).expect("the rating climbs in");
    });
    crate::field_raiders_rig::seat_npc(&mut rig, escort, mendes);
    // The captain is set down at Ceres while her rig is out.
    staff_hand(&mut rig, |ctx| {
        ctx.w.move_to(who, pool).expect("the captain is at Ceres");
    });

    let said = run(
        &mut rig.tw,
        who,
        &format!("+sortie/discharge #{}", mendes.0),
    );
    assert!(
        said.contains(&format!("Ilya Mendes{}", tail(game::ON_SORTIE))),
        "out of the bay: {said}"
    );
    let contract = run(&mut rig.tw, who, &format!("+sortie/contract #{}", mendes.0));
    assert!(contract.contains("Crew contract"), "{contract}");
    assert!(npc_exit_of(rig.tw.world(), mendes).is_none());
    assert_eq!(
        room_of(&rig, mendes),
        escort,
        "the rating is still in the rig"
    );
}

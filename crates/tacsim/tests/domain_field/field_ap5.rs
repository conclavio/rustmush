//! **Doc 45 unit AP.5 — absent at admission** (Q-45-1, Q-45-2; doc 44's Q-44-35, Q-44-36,
//! Q-44-37, Q-44-43, Q-44-46), over the composed world (`driving`'s keyed rig on Ahuna Flank)
//! with the arena's egress pumped by hand.
//!
//! Admission asks the connection as well as the seat. Every story is a field of three parties
//! or more, and a player's presence is the engine's own (`field_mw7`'s helpers: a lost link is a
//! `ConnDown` with `Lost`, a quit a `ConnDown` with `Quit`, a session's end a `SessionGone`).
//!
//! 1. [`ap5_a_player_logged_off_in_a_rigs_seat_is_flown_home_by_its_mind`]: a miner's
//!    prospector, a leased escort whose pilot quit in its cab, and an intruder whose fire order
//!    opens the field (`escort_lease`'s ground). The escort is its mind's from the turn the
//!    field takes it in: it fires nothing, drives to its carrier's ramp, is let out and is put
//!    aboard. Its pilot reads why at login.
//! 2. [`ap5_a_mark_left_by_a_player_who_then_quit_is_not_fired_on`]: the ward's pilot marks the
//!    raider's flight Hostile, which opens the field, and quits before it is admitted. The
//!    ward's party holds the raider Hostile eight cells off from the first pulse and the ward
//!    fires nothing. The pilot reconnects while the field runs, is seated again, finds the
//!    mark still standing, and their first command is what the hull does.
//! 3. [`ap5_a_connected_pilot_is_admitted_as_before`]: nobody present loses anything, a `DARK`
//!    character included.
//! 4. [`ap5_a_link_lost_before_the_field_opens_keeps_what_is_left_of_its_five_minutes`]: a link
//!    lost a hundred seconds before the field opens. The seat is the pilot's, link-dead, and
//!    the unit passes to its mind five minutes after the loss, not after the opening.
//! 5. [`ap5_a_pilot_who_is_down_or_held_is_admitted_under_the_mind`]: a seated character who is
//!    incapacitated, and one who is held, both connected (Q-44-43).
//! 6. [`ap5_a_restart_before_the_player_returns_keeps_the_mind_and_still_seats_them`].
//!
//! What the stories stand in for: the yard's three hulls are fixture hulls with players seated
//! (CF.2's `armed` and `seat`), and story 5's crew records are written through the crew book's
//! test seam, since no command leaves a character incapacitated or held in a seat on open
//! ground.

use mush_core::ObjId;
use tacsim::arena::ledger::LedgerEvent;
use tacsim::field::absent::{Away, FIELD_LINKDEAD_GRACE_WALL_S, Why, away_of, minded};

use crate::driving_rig::{
    Rig, cid, command, crash, land, on_pad_as, run, staff_hand, stocked_tuned, to_ic, warm,
};
use crate::escort_lease::{fired, in_field, stand_up};
use crate::field_host_rig::{AHUNA, armed, egress, frame, seat, second, site_record, task, until};
use crate::field_mw7::{
    ESCORT_AT, RAIDER_AT, WARD_AT, Yard, bound, connect, end_session, fired_at, hostiles,
    lose_link, quiet, quit,
};

/// The three hulls of `field_mw7`'s yard and who sits in each, before any field.
struct Cast {
    pilot: ObjId,
    ward: ObjId,
    captain: ObjId,
    escort: ObjId,
    stranger: ObjId,
    raider: ObjId,
}

/// **Three parties of one hull each on Ahuna Flank's open ground**: the ward, a raider eight
/// cells from it, and a third captain's escort to the west. Every player is connected, as
/// the rig spawns them.
fn cast(rig: &mut Rig) -> Cast {
    let (pilot, _, _) = on_pad_as(rig, "Pilot", 1, "Salt Moth", 0);
    let ward = armed(rig, "Ward Dog", WARD_AT, 180_000);
    seat(rig, ward, pilot);
    let (captain, _, _) = on_pad_as(rig, "Other", 2, "Tally Moth", 0);
    let (stranger, _, _) = on_pad_as(rig, "Third", 3, "Lone Moth", 0);
    let escort = armed(rig, "Red Hound", ESCORT_AT, 90_000);
    let raider = armed(rig, "Grey Hound", RAIDER_AT, 0);
    seat(rig, escort, captain);
    seat(rig, raider, stranger);
    warm(rig);
    for _ in 0..3 {
        let _ = second(rig);
    }
    for who in [pilot, captain, stranger] {
        assert!(
            rig.tw.world().is_connected(who) && rig.tw.world().has_live_conns(who),
            "{who} is connected over a live transport"
        );
    }
    Cast {
        pilot,
        ward,
        captain,
        escort,
        stranger,
        raider,
    }
}

/// `who` types the line that promotes the site, and the promotion lands.
fn promote(rig: &mut Rig, who: ObjId, line: &str) {
    let (out, queued) = command(rig, who, line);
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    land(rig, &queued[0]).expect("the promotion lands");
}

/// The promoted site is admitted: the field's task opens with the three parties in it.
async fn field(mut rig: Rig, cast: &Cast) -> Yard {
    let mut rx = egress(&mut rig);
    let mut lines = Vec::new();
    assert!(
        until(&mut rig, &mut rx, &mut lines, 15, |rig, _| task(rig)
            .is_some())
        .await,
        "the field's task opens"
    );
    let live = tacsim::field::live_on(rig.tw.world(), &cid(AHUNA)).expect("the live record");
    let parties: std::collections::BTreeSet<_> = [cast.ward, cast.raider, cast.escort]
        .into_iter()
        .filter_map(|unit| live.party_of(unit))
        .collect();
    assert_eq!(parties.len(), 3, "three parties stand in the field");
    Yard {
        rig,
        pilot: cast.pilot,
        ward: cast.ward,
        stranger: cast.stranger,
        raider: cast.raider,
        captain: cast.captain,
        escort: cast.escort,
        rx,
        lines,
    }
}

/// Who the simulation's seat assignment names in `unit`.
fn sits_in(rig: &Rig, unit: ObjId) -> Option<ObjId> {
    rig.tw
        .world()
        .module_state::<mush_sim::SpatialState>()
        .and_then(|state| state.assignments.get(&unit))
        .and_then(|host| host.seats.values().next())
        .map(|seat| seat.actor)
}

/// The record of a unit a field took in with its pilot away.
fn admitted_away(rig: &Rig, unit: ObjId) -> Option<(Why, bool)> {
    away_of(rig.tw.world(), unit).map(|away| (away.why, away.handed))
}

/// **Story 1** (this module's head): a player logged off in a rig's seat is not admitted as its
/// pilot. The rig is its mind's, goes home and never fires.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn ap5_a_player_logged_off_in_a_rigs_seat_is_flown_home_by_its_mind() {
    let mut rig = stocked_tuned(quiet);
    // A miner's prospector at work, the escort out beside it, the intruder to the east.
    let (escort, hull, intruder, hostile) = stand_up(&mut rig);
    // The escort's pilot logs off in the cab, on open ground. No field runs, so nothing is
    // handed over, and the seat still names them.
    quit(&mut rig, escort);
    assert!(!rig.tw.world().is_connected(escort));
    assert_eq!(
        sits_in(&rig, hull),
        Some(escort),
        "the seat outlives the quit"
    );
    assert_eq!(
        away_of(rig.tw.world(), hull),
        None,
        "no field holds the rig"
    );
    let mut rx = egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();
    for _ in 0..3 {
        let _ = second(&mut rig);
    }
    // The intruder's fire order opens the field.
    promote(&mut rig, intruder, "+field/fire W");
    assert!(
        until(&mut rig, &mut rx, &mut lines, 15, |rig, _| task(rig)
            .is_some())
        .await,
        "the field's task opens"
    );
    let live = tacsim::field::live_on(rig.tw.world(), &cid(AHUNA)).expect("the live record");
    let parties: std::collections::BTreeSet<_> = live.units.iter().map(|row| row.party).collect();
    assert!(
        parties.len() >= 3,
        "the miner, the escort and the intruder are three parties: {parties:?}"
    );
    assert!(in_field(&rig, hull), "the escort is in the field");
    // Admission asked the connection: the rig is its mind's from the turn it was taken in.
    assert_eq!(
        admitted_away(&rig, hull),
        Some((Why::AwayAtAdmission, true)),
        "a pilot who is not connected is not admitted as the rig's pilot"
    );
    assert!(
        minded(rig.tw.world(), hull) && !bound(&rig, escort),
        "the mind has the rig and nobody holds its seat"
    );
    // It drives to the ramp, is let out, and the host puts it aboard.
    let out = until(&mut rig, &mut rx, &mut lines, 180, |rig, _| {
        !in_field(rig, hull) && site_record(rig).is_some_and(|record| record.exited(hull))
    })
    .await;
    assert!(
        out,
        "the mind takes the escort out at its carrier's ramp: pose {:?}, away {:?}",
        mush_sim::pose_of(rig.tw.world(), hull)
            .map(|(_, pose)| tacsim::surface::excursion::site_cell(pose.x_cm, pose.y_cm)),
        away_of(rig.tw.world(), hull)
    );
    let home = until(&mut rig, &mut rx, &mut lines, 120, |rig, _| {
        let due = {
            let h = crate::driving_rig::handles(&rig.tw);
            crate::driving_rig::typed(
                &mut rig.tw,
                &h.families.excursions,
                &tacsim::surface::excursion::RigKey(hull),
            )
            .and_then(|record| record.act.map(|act| act.due_ic))
        };
        if let Some(due) = due {
            to_ic(rig, due);
        }
        matches!(
            crate::driving_rig::parent(&rig.tw, hull),
            mush_sim::SpatialParent::Interior { .. }
        )
    })
    .await;
    assert!(home, "the host puts the escort aboard its carrier");
    // The black box, caught up: not one round of its own, with the intruder in the field.
    let _ = until(&mut rig, &mut rx, &mut lines, 5, |_, _| false).await;
    assert_eq!(
        fired(&lines, hull),
        0,
        "the mind never fired: {} round(s) at the intruder",
        fired_at(&lines, hull, hostile)
    );
    // The pilot reads why at login, in the reconnect queue AP.1 uses.
    let heard = connect(&mut rig, escort);
    assert!(
        heard.contains(tacsim::port_tick::NOTICE_HEADING)
            && heard.contains(&format!(
                "A field opened around #{} while you were away",
                hull.0
            )),
        "the returning pilot reads what became of the rig: {heard}"
    );
}

/// **Story 2** (this module's head): a mark left by a player who then quit is not fired on, and
/// the same player, reconnecting while the field runs, is seated again and their first command
/// wins.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn ap5_a_mark_left_by_a_player_who_then_quit_is_not_fired_on() {
    // A hull released to a mind under no rule falls back first and rallies after
    // `[tacsim.arena.reflex].rally_ms`, then fights whoever its party holds Hostile (measured:
    // with the kernel never told the ward's player was away, the ward put fourteen rounds into
    // the raider). The rally is shortened here so the quiet stretch below is well past it.
    let mut rig = stocked_tuned(|cfg| {
        quiet(cfg);
        cfg.arena.reflex.rally_ms = 4_000;
    });
    let cast = cast(&mut rig);
    let (pilot, ward, raider) = (cast.pilot, cast.ward, cast.raider);
    // The ward's pilot marks the raider's flight Hostile (it bears north on the presence
    // picture, eight cells off), which promotes the site, and
    // quits before the field's tick admits anyone.
    promote(&mut rig, pilot, "+field/mark N=hostile");
    quit(&mut rig, pilot);
    // A bare connection (an SSH `mind push`) is no presence: it changes nothing below.
    {
        let mut out = mush_core::Outbox::new();
        let mut fx = mush_core::Effects::new();
        rig.tw.engine_mut().step(
            mush_core::Inbound::BareUp {
                conn: 9_001,
                actor: pilot,
            },
            &mut out,
            &mut fx,
        );
    }
    assert!(!rig.tw.world().is_connected(pilot));
    let mut yard = field(rig, &cast).await;
    assert_eq!(
        away_of(yard.rig.tw.world(), ward),
        Some(Away {
            why: Why::AwayAtAdmission,
            since_wall_s: away_of(yard.rig.tw.world(), ward).map_or(0, |away| away.since_wall_s),
            handed: true,
        }),
        "the ward is its mind's from admission"
    );
    assert!(minded(yard.rig.tw.world(), ward) && !bound(&yard.rig, pilot));
    // Thirty seconds, seven rallies' worth, with the raider its party marked Hostile eight
    // cells off: nothing.
    let _ = yard.wait(30, |_, _| false).await;
    assert_eq!(
        fired(&yard.lines, ward),
        0,
        "the mind opens fire on nobody, whatever its pilot marked: {}",
        yard.exchange()
    );
    assert!(
        in_field(&yard.rig, ward) && minded(yard.rig.tw.world(), ward),
        "no carrier of its party is on the site, so it holds: {}",
        yard.exchange()
    );
    // The pilot reconnects while the field runs: seated again.
    let heard = connect(&mut yard.rig, pilot);
    assert!(
        heard.contains("You are back at the controls"),
        "the returning pilot is told: {heard}"
    );
    assert!(
        away_of(yard.rig.tw.world(), ward).is_none() && bound(&yard.rig, pilot),
        "the pilot is seated again: {:?}",
        away_of(yard.rig.tw.world(), ward)
    );
    let focus = run(
        &mut yard.rig.tw,
        pilot,
        &format!("+focus #{}/cockpit", ward.0),
    );
    // The mark stood the whole time: the first frame of the new tenure shows the raider Hostile.
    assert!(
        yard.wait(30, |rig, _| hostiles(rig, pilot) > 0).await,
        "the returning pilot's frame holds the raider Hostile: {focus}; {:?}",
        frame(&yard.rig, pilot).map(|frame| frame
            .contacts
            .iter()
            .map(|contact| (contact.cell, contact.relation))
            .collect::<Vec<_>>())
    );
    assert_eq!(
        fired(&yard.lines, ward),
        0,
        "and the ward fired on it at no point: {} round(s) at the raider",
        fired_at(&yard.lines, ward, raider)
    );
    // Their first command is what the hull does.
    let drive = run(&mut yard.rig.tw, pilot, "DRIVE 150,110");
    let driving = yard
        .wait(60, |rig, _| {
            frame(rig, pilot).is_some_and(|frame| {
                frame.order.is_none()
                    && frame
                        .motor
                        .as_ref()
                        .is_some_and(|motor| motor.requested_goal == (150, 110))
            })
        })
        .await;
    assert!(
        driving,
        "the pilot's DRIVE is the hull's goal, and the mind's order is gone: focus {focus}; drive {drive}; {:?}",
        frame(&yard.rig, pilot).map(|frame| (frame.cell, frame.order, frame.motor))
    );
}

/// **Story 3** (this module's head): a connected pilot is admitted as before, a `DARK` one
/// included.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ap5_a_connected_pilot_is_admitted_as_before() {
    let mut rig = stocked_tuned(quiet);
    let cast = cast(&mut rig);
    let (pilot, ward, captain, escort) = (cast.pilot, cast.ward, cast.captain, cast.escort);
    // `DARK` hides a character from a room; it does not take them from their controls.
    staff_hand(&mut rig, |ctx| {
        ctx.w
            .set_flag(captain, mush_core::Flags::DARK, true)
            .expect("the flag");
    });
    promote(&mut rig, captain, "+field/fire E");
    let mut yard = field(rig, &cast).await;
    for (who, unit) in [
        (pilot, ward),
        (captain, escort),
        (cast.stranger, cast.raider),
    ] {
        assert_eq!(
            away_of(yard.rig.tw.world(), unit),
            None,
            "#{} carries no presence record",
            unit.0
        );
        assert!(
            bound(&yard.rig, who) && !minded(yard.rig.tw.world(), unit),
            "{who} holds the seat of #{}",
            unit.0
        );
    }
    assert!(
        yard.wait(30, |rig, _| frame(rig, pilot).is_some()).await,
        "the pilot's cockpit frame arrives"
    );
    let drive = run(&mut yard.rig.tw, pilot, "DRIVE 150,110");
    let driving = yard
        .wait(60, |rig, _| {
            frame(rig, pilot).is_some_and(|frame| {
                frame
                    .motor
                    .as_ref()
                    .is_some_and(|motor| motor.requested_goal == (150, 110))
            })
        })
        .await;
    assert!(driving, "the pilot flies the ward: {drive}");
}

/// **Story 4** (this module's head): a link lost before the field opens keeps what is left of
/// its five minutes, and no more.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ap5_a_link_lost_before_the_field_opens_keeps_what_is_left_of_its_five_minutes() {
    let mut rig = stocked_tuned(quiet);
    let cast = cast(&mut rig);
    let (pilot, ward) = (cast.pilot, cast.ward);
    // The link is lost on open ground, a hundred seconds before anything opens.
    lose_link(&mut rig, pilot);
    let lost = rig.tw.world().now();
    assert!(
        rig.tw.world().is_connected(pilot) && !rig.tw.world().has_live_conns(pilot),
        "the session is link-dead"
    );
    for _ in 0..100 {
        let _ = second(&mut rig);
    }
    promote(&mut rig, cast.captain, "+field/fire E");
    let mut yard = field(rig, &cast).await;
    // The seat is the pilot's, link-dead, and the clock is the one that started at the loss.
    assert_eq!(
        away_of(yard.rig.tw.world(), ward),
        Some(Away {
            why: Why::LinkLost,
            since_wall_s: lost,
            handed: false,
        }),
        "the five minutes run from the loss, not from the opening"
    );
    assert!(
        bound(&yard.rig, pilot) && !minded(yard.rig.tw.world(), ward),
        "inside the five minutes the seat is still the pilot's"
    );
    // The server's own grace ends the session; the field's clock is unmoved by it.
    end_session(&mut yard.rig, pilot);
    let left = lost + FIELD_LINKDEAD_GRACE_WALL_S - yard.rig.tw.world().now();
    assert!(
        (20..FIELD_LINKDEAD_GRACE_WALL_S - 100).contains(&left),
        "well under five minutes are left: {left}"
    );
    yard.pass(left - 10);
    assert!(
        !minded(yard.rig.tw.world(), ward),
        "ten seconds short of five minutes from the loss the mind has nothing: {:?}",
        away_of(yard.rig.tw.world(), ward)
    );
    yard.pass(15);
    assert_eq!(
        admitted_away(&yard.rig, ward),
        Some((Why::LinkLost, true)),
        "five minutes after the loss the mind has the unit"
    );
    assert!(!bound(&yard.rig, pilot));
    assert_eq!(fired(&yard.lines, ward), 0);
}

/// **Story 5** (this module's head): a seated character who is incapacitated, or held, is not
/// at the controls, connected or not (Q-44-43).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ap5_a_pilot_who_is_down_or_held_is_admitted_under_the_mind() {
    use tacsim::field::crew::{CREW_FORMAT, CrewCondition, CrewLocation, CrewRecord};
    let mut rig = stocked_tuned(quiet);
    let cast = cast(&mut rig);
    let (pilot, ward, stranger, raider) = (cast.pilot, cast.ward, cast.stranger, cast.raider);
    // The ward's pilot is down in the hull; the raider's is held. Both are connected.
    let escort = cast.escort;
    staff_hand(&mut rig, |ctx| {
        let now = ctx.w.world_ic_now().at;
        let record = |character, location, condition| CrewRecord {
            format: CREW_FORMAT,
            character,
            location,
            condition,
            at_ic: now,
            organizer: None,
            extraction: None,
            treatment: None,
            resolve_permille: None,
        };
        tacsim::field::crew::hold_for_tests(
            ctx.w,
            record(
                pilot,
                CrewLocation::Unit { unit: ward },
                CrewCondition::Incapacitated,
            ),
        );
        tacsim::field::crew::hold_for_tests(
            ctx.w,
            record(
                stranger,
                CrewLocation::Captured {
                    carrier: escort,
                    captor: cast.captain,
                    since_ic: now,
                },
                CrewCondition::Fit,
            ),
        );
    });
    promote(&mut rig, cast.captain, "+field/fire E");
    let mut yard = field(rig, &cast).await;
    for (who, unit) in [(pilot, ward), (stranger, raider)] {
        assert!(yard.rig.tw.world().is_connected(who), "{who} is connected");
        assert_eq!(
            admitted_away(&yard.rig, unit),
            Some((Why::Incapacitated, true)),
            "#{} is admitted under its mind",
            unit.0
        );
        assert!(!bound(&yard.rig, who), "{who} holds no seat");
    }
    // A command typed meanwhile takes no seat back.
    let drive = run(&mut yard.rig.tw, pilot, "DRIVE 150,110");
    let _ = yard.wait(5, |_, _| false).await;
    assert!(
        !bound(&yard.rig, pilot) && minded(yard.rig.tw.world(), ward),
        "a DRIVE from a pilot who is down leaves the seat released: {drive}"
    );
    // Nor does a reconnect give the controls back to a pilot who is not fit.
    lose_link(&mut yard.rig, pilot);
    end_session(&mut yard.rig, pilot);
    let heard = connect(&mut yard.rig, pilot);
    assert!(
        heard.contains("You are in no state to fly"),
        "the returning pilot is told why: {heard}"
    );
    assert!(!bound(&yard.rig, pilot) && minded(yard.rig.tw.world(), ward));
    assert_eq!(fired(&yard.lines, ward) + fired(&yard.lines, raider), 0);
}

/// **Story 6** (this module's head): a restart between admission and the player's return keeps
/// the unit under its mind, and the player who then connects is seated.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ap5_a_restart_before_the_player_returns_keeps_the_mind_and_still_seats_them() {
    let mut rig = stocked_tuned(quiet);
    let cast = cast(&mut rig);
    let (pilot, ward) = (cast.pilot, cast.ward);
    // The pilot's session ended long before: a lost link, then the server's grace.
    lose_link(&mut rig, pilot);
    end_session(&mut rig, pilot);
    for _ in 0..FIELD_LINKDEAD_GRACE_WALL_S + 5 {
        let _ = second(&mut rig);
    }
    promote(&mut rig, cast.captain, "+field/fire E");
    let mut yard = field(rig, &cast).await;
    assert_eq!(
        admitted_away(&yard.rig, ward),
        Some((Why::AwayAtAdmission, true)),
        "a pilot gone longer than five minutes is not waited for"
    );
    // The field saves with the mind holding the unit, then the server goes down and comes up.
    let _ = yard.wait(3, |_, _| false).await;
    let Yard {
        rig, mut rx, lines, ..
    } = yard;
    assert_eq!(fired(&lines, ward), 0, "nothing fired before the restart");
    let mut rig = crash(rig, &mut rx, &[cast.captain, cast.stranger]).await;
    let mut rx = egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();
    assert!(
        until(&mut rig, &mut rx, &mut lines, 30, |rig, _| task(rig)
            .is_some())
        .await,
        "the field resumes"
    );
    assert_eq!(
        admitted_away(&rig, ward),
        Some((Why::AwayAtAdmission, true)),
        "the record survives the restart"
    );
    assert!(
        minded(rig.tw.world(), ward) && !bound(&rig, pilot),
        "the boot seats nobody who is not there"
    );
    let _ = until(&mut rig, &mut rx, &mut lines, 8, |_, _| false).await;
    assert_eq!(fired(&lines, ward), 0, "the resumed mind fires on nobody");
    // The pilot connects and is seated.
    let heard = connect(&mut rig, pilot);
    assert!(
        heard.contains("You are back at the controls"),
        "the returning pilot is told: {heard}"
    );
    assert!(
        away_of(rig.tw.world(), ward).is_none() && bound(&rig, pilot),
        "the pilot is seated again after the restart"
    );
}

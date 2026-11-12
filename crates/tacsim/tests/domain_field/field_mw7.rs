//! **Doc 44 unit MW.7 — the field's backup mind and the absent-player gate** (Q-44-16 to
//! Q-44-19, Q-44-22, Q-44-35 to Q-44-38, Q-44-42, Q-44-43, Q-44-46; doc 35 §11.4), over the
//! composed world (`driving`'s keyed rig on Ahuna Flank) with the arena's egress pumped by hand.
//!
//! Every story is a field of three parties. A player's presence is the engine's own: the story
//! connects the pilot, then drops the connection as a lost link (`ConnDown`, `Lost`), ends the
//! session (`SessionGone`) or quits (`ConnDown`, `Quit`), and the seat's hooks do the rest. The
//! five minutes are the field's own clock on the world's wall second, which the rig steps a
//! second at a time, so no story waits five real minutes.
//!
//! 1. [`mw7_the_house_mind_takes_an_absent_pilots_rig_home_and_never_fires_first`]: a miner's
//!    prospector, a leased escort flown from its own cab and an intruder (`escort_lease`'s
//!    ground). The escort's pilot sets weapons free with the intruder held Hostile in view and
//!    loses their link. Inside the five minutes the unit is its pilot's still; at five minutes
//!    it passes to its mind, which fires nothing, drives to the ramp, is let out by the kernel
//!    and is put aboard its carrier by the host.
//! 2. [`mw7_an_absent_pilots_mind_answers_only_the_unit_that_fired_on_it`]: a clean QUIT hands
//!    the unit over at once. With a party marked Hostile in range and weapons free it fires on
//!    nobody, and once the raider's fire arrives on it, it answers the raider and nobody else.
//!    Its party has no carrier on the site, so it holds where it stands (Q-44-42).
//! 3. [`mw7_a_hold_fire_left_by_the_pilot_holds_through_an_attack`]: the same, the seat left at
//!    weapons hold: under the raider's fire it answers nobody.
//! 4. [`mw7_a_pilot_who_reconnects_ten_minutes_later_is_back_in_the_seat`]: the link lost, the
//!    session ended, ten minutes gone. The pilot connects, is seated again, and their first
//!    command is what the hull does.
//! 5. [`mw7_a_pilot_knocked_out_while_connected_stops_steering`]: a pilot inside a hull whose
//!    cockpit is destroyed loses the controls at once, connected or not, and a reconnect does
//!    not give them back.
//! 6. [`mw7_a_restart_while_the_mind_holds_the_unit_resumes_the_mind`]: the server restarts
//!    with the mind holding the unit. The pilot is not seated by the boot; the mind still
//!    answers only who fires on it; the pilot connects and is seated.
//!
//! What the stories stand in for: the raider and the ward are fixture hulls with players seated
//! (CF.2's `armed` and `seat`), and the raider's player types every shot. The filed mind and the
//! faulting mind are pinned in the arena (`domain_arena`), where a script can be sealed without
//! a Registry rating.

use mush_core::ObjId;
use tacsim::arena::ledger::{EventKind, LedgerEvent};
use tacsim::arena::party::Relation;
use tacsim::arena_task::ArenaOut;
use tacsim::field::absent::{Away, FIELD_LINKDEAD_GRACE_WALL_S, Why, away_of, minded};
use tokio::sync::mpsc::Receiver;

use crate::driving_rig::{Rig, cid, command, crash, land, on_pad_as, run, stocked_tuned, to_ic};
use crate::escort_lease::{contact_near, fired, in_field, stand_up};
use crate::field_host_rig::{
    AHUNA, armed, egress, frame, pump, seat, second, site_record, task, tune, until, weapon_of,
};

/// The field holds through the stories' quiet stretches.
fn quiet(cfg: &mut tacsim::TacsimConfig) {
    tune(cfg);
    cfg.field.disengage_s = 3_600;
    cfg.field.idle_demote_s = 3_600;
}

/// Drop every connection of `who` with `reason`.
fn drop_conns(rig: &mut Rig, who: ObjId, reason: mush_core::DisconnectKind) {
    let mut out = mush_core::Outbox::new();
    let mut fx = mush_core::Effects::new();
    let conns: Vec<u64> = (1..=64)
        .filter(|conn| rig.tw.engine().actor_of(*conn) == Some(who))
        .collect();
    assert!(!conns.is_empty(), "{who} holds a connection");
    for conn in conns {
        rig.tw.engine_mut().step(
            mush_core::Inbound::ConnDown {
                conn,
                actor: who,
                reason,
            },
            &mut out,
            &mut fx,
        );
    }
}

/// `who`'s link is lost: the server's own grace starts, and the field's.
fn lose_link(rig: &mut Rig, who: ObjId) {
    drop_conns(rig, who, mush_core::DisconnectKind::Lost);
}

/// `who` quits.
fn quit(rig: &mut Rig, who: ObjId) {
    drop_conns(rig, who, mush_core::DisconnectKind::Quit);
}

/// The server's grace runs out for `who`: their session ends.
fn end_session(rig: &mut Rig, who: ObjId) {
    let mut out = mush_core::Outbox::new();
    let mut fx = mush_core::Effects::new();
    rig.tw.engine_mut().step(
        mush_core::Inbound::SessionGone { actor: who },
        &mut out,
        &mut fx,
    );
}

/// What `who` hears when they connect.
fn connect(rig: &mut Rig, who: ObjId) -> String {
    rig.tw
        .connect(who)
        .into_iter()
        .filter_map(|(_, event)| event.text().map(str::to_owned))
        .collect::<Vec<_>>()
        .join("\n")
}

/// `secs` world seconds, the arena's egress pumped as they pass.
fn pass(rig: &mut Rig, rx: &mut Receiver<ArenaOut>, lines: &mut Vec<LedgerEvent>, secs: i64) {
    for n in 0..secs {
        let _ = second(rig);
        if n % 10 == 0 {
            let _ = pump(rig, rx, lines);
        }
    }
    let _ = pump(rig, rx, lines);
}

/// Whether the arena's registry holds a seat for `who`.
fn bound(rig: &Rig, who: ObjId) -> bool {
    rig.tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .is_some_and(|registry| registry.seat_of_pilot(who).is_some())
}

/// The rounds `shooter` fired at `target`.
fn fired_at(lines: &[LedgerEvent], shooter: ObjId, target: ObjId) -> usize {
    lines
        .iter()
        .filter(|line| {
            matches!(&line.kind, EventKind::Fired { unit, target: at, .. }
                if *unit == shooter && *at == target)
        })
        .count()
}

/// The rounds of `shooter` that arrived on `target`, hit or miss.
fn arrived(lines: &[LedgerEvent], shooter: ObjId, target: ObjId) -> usize {
    lines
        .iter()
        .filter(|line| {
            matches!(&line.kind, EventKind::Impact { unit, target: at, .. }
                if *unit == shooter && *at == target)
        })
        .count()
}

/// How many live Hostile contacts `who`'s own frame holds.
fn hostiles(rig: &Rig, who: ObjId) -> usize {
    frame(rig, who).map_or(0, |frame| {
        frame
            .contacts
            .iter()
            .filter(|contact| contact.relation == Relation::Hostile)
            .count()
    })
}

/// **Three parties of one hull each on Ahuna Flank, in a running field**: the ward, whose pilot
/// the story takes away; a raider eight cells south of it; an escort of a third captain's, whose
/// fire order on the raider promotes the site. The ward's pilot is connected and has marked the
/// raider's party Hostile.
struct Yard {
    rig: Rig,
    /// The ward's pilot and hull.
    pilot: ObjId,
    ward: ObjId,
    /// The raider's player and hull.
    stranger: ObjId,
    raider: ObjId,
    /// The escort's captain and hull.
    captain: ObjId,
    escort: ObjId,
    rx: Receiver<ArenaOut>,
    lines: Vec<LedgerEvent>,
}

/// The cells the three stand on.
const WARD_AT: (i64, i64) = (146, 114);
const RAIDER_AT: (i64, i64) = (146, 122);
const ESCORT_AT: (i64, i64) = (136, 122);

/// [`Yard`], with `ward` the hull the story stood up for the ward's pilot.
async fn yard_with(mut rig: Rig, pilot: ObjId, ward: ObjId) -> Yard {
    let (captain, _, _) = on_pad_as(&mut rig, "Other", 2, "Tally Moth", 0);
    let (stranger, _, _) = on_pad_as(&mut rig, "Third", 3, "Lone Moth", 0);
    let escort = armed(&mut rig, "Red Hound", ESCORT_AT, 90_000);
    let raider = armed(&mut rig, "Grey Hound", RAIDER_AT, 0);
    seat(&mut rig, escort, captain);
    seat(&mut rig, raider, stranger);
    crate::driving_rig::warm(&mut rig);
    let _ = rig.tw.connect(pilot);
    let mut rx = egress(&mut rig);
    let mut lines = Vec::new();
    for _ in 0..3 {
        let _ = second(&mut rig);
    }
    let (out, queued) = command(&mut rig, captain, "+field/fire E");
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    land(&mut rig, &queued[0]).expect("the promotion lands");
    assert!(
        until(&mut rig, &mut rx, &mut lines, 15, |rig, _| task(rig)
            .is_some())
        .await,
        "the field's task opens"
    );
    let live = tacsim::field::live_on(rig.tw.world(), &cid(AHUNA)).expect("the live record");
    let parties: std::collections::BTreeSet<_> = [ward, raider, escort]
        .into_iter()
        .filter_map(|unit| live.party_of(unit))
        .collect();
    assert_eq!(parties.len(), 3, "three parties stand in the field");
    // The ward's pilot leads a party of one, and marks the raider's.
    let letter = live.party_of(raider).expect("the raider's party").letter();
    let marked = run(&mut rig.tw, pilot, &format!("+field/mark {letter}=hostile"));
    assert!(
        until(&mut rig, &mut rx, &mut lines, 30, |rig, _| hostiles(
            rig, pilot
        ) > 0)
        .await,
        "the ward's frame holds the raider Hostile: {marked}; {:?}",
        frame(&rig, pilot).map(|frame| frame
            .contacts
            .iter()
            .map(|contact| (contact.cell, contact.relation))
            .collect::<Vec<_>>())
    );
    Yard {
        rig,
        pilot,
        ward,
        stranger,
        raider,
        captain,
        escort,
        rx,
        lines,
    }
}

/// [`Yard`] with the ward a fixture hull its pilot flies from a console.
async fn yard() -> Yard {
    let mut rig = stocked_tuned(quiet);
    let (pilot, _, _) = on_pad_as(&mut rig, "Pilot", 1, "Salt Moth", 0);
    let ward = armed(&mut rig, "Ward Dog", WARD_AT, 180_000);
    seat(&mut rig, ward, pilot);
    yard_with(rig, pilot, ward).await
}

impl Yard {
    /// Pump and tick until `done` holds or `secs` of wall time pass.
    async fn wait(&mut self, secs: u64, done: impl FnMut(&mut Rig, &[LedgerEvent]) -> bool) -> bool {
        until(&mut self.rig, &mut self.rx, &mut self.lines, secs, done).await
    }

    /// World seconds, with nothing waited for.
    fn pass(&mut self, secs: i64) {
        pass(&mut self.rig, &mut self.rx, &mut self.lines, secs);
    }

    /// **The raider's player keeps a lock on the ward and fires** until `done` holds or `secs`
    /// of wall time pass: the two lines a player types, once a wall second.
    async fn raid(&mut self, secs: u64, mut done: impl FnMut(&Rig, &[LedgerEvent]) -> bool) -> bool {
        let gun = weapon_of(&self.rig, self.raider);
        let at = (
            u16::try_from(WARD_AT.0).expect("a cell"),
            u16::try_from(WARD_AT.1).expect("a cell"),
        );
        for _ in 0..secs {
            if let Some(contact) = contact_near(&self.rig, self.stranger, at)
                && frame(&self.rig, self.stranger).is_some_and(|frame| frame.lock != Some(contact.id))
            {
                let _ = run(
                    &mut self.rig.tw,
                    self.stranger,
                    &format!("LOCK {}", contact.designator),
                );
            }
            let _ = run(&mut self.rig.tw, self.stranger, &format!("FIRE {gun}"));
            if self.wait(1, |rig, lines| done(rig, lines)).await {
                return true;
            }
        }
        false
    }

    /// The raider's player ceases fire.
    fn cease(&mut self) {
        let gun = weapon_of(&self.rig, self.raider);
        let _ = run(&mut self.rig.tw, self.stranger, &format!("FIRE {gun} OFF"));
    }

    /// What the black box holds of the ward and the raider, for a failed wait.
    fn exchange(&self) -> String {
        format!(
            "ward fired {} (at the raider {}, at the escort {}); raider fired {}, arrived on the ward {}; away {:?}",
            fired(&self.lines, self.ward),
            fired_at(&self.lines, self.ward, self.raider),
            fired_at(&self.lines, self.ward, self.escort),
            fired(&self.lines, self.raider),
            arrived(&self.lines, self.raider, self.ward),
            away_of(self.rig.tw.world(), self.ward),
        )
    }
}

/// **Story 1** (this module's head): the house mind takes an absent pilot's rig home, with a
/// Hostile in view, and never fires first.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn mw7_the_house_mind_takes_an_absent_pilots_rig_home_and_never_fires_first() {
    let mut rig = stocked_tuned(quiet);
    // A miner's prospector at work, the escort out beside it, the intruder to the east.
    let (escort, hull, _intruder, hostile) = stand_up(&mut rig);
    let _ = rig.tw.connect(escort);
    let mut rx = egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();
    for _ in 0..3 {
        let _ = second(&mut rig);
    }
    let (out, queued) = command(&mut rig, escort, "+field/fire E");
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    land(&mut rig, &queued[0]).expect("the promotion lands");
    assert!(
        until(&mut rig, &mut rx, &mut lines, 15, |rig, _| task(rig)
            .is_some())
        .await,
        "the field's task opens"
    );
    let live = tacsim::field::live_on(rig.tw.world(), &cid(AHUNA)).expect("the live record");
    let parties: std::collections::BTreeSet<_> =
        live.units.iter().map(|row| row.party).collect();
    assert!(
        parties.len() >= 3,
        "the miner, the escort and the intruder are three parties: {parties:?}"
    );
    // Weapons free, and the intruder held Hostile in the escort's own sight.
    let free = run(&mut rig.tw, escort, "FIRE FREE");
    assert!(
        until(&mut rig, &mut rx, &mut lines, 30, |rig, _| {
            frame(rig, escort).is_some_and(|frame| frame.weapons_free) && hostiles(rig, escort) > 0
        })
        .await,
        "the escort is weapons free with a Hostile in view: {free}; {:?}",
        frame(&rig, escort).map(|frame| (frame.weapons_free, frame.contacts.len()))
    );
    // Its opening shot has left and landed by now; nothing of its own is armed.
    let _ = until(&mut rig, &mut rx, &mut lines, 10, |_, _| false).await;
    let before = fired(&lines, hull);

    // The link is lost. Inside the five minutes the unit is its pilot's: the seat is linkdead.
    lose_link(&mut rig, escort);
    assert_eq!(
        away_of(rig.tw.world(), hull).map(|away| (away.why, away.handed)),
        Some((Why::LinkLost, false)),
        "the field's clock starts with the lost link"
    );
    pass(
        &mut rig,
        &mut rx,
        &mut lines,
        FIELD_LINKDEAD_GRACE_WALL_S - 20,
    );
    assert!(
        !minded(rig.tw.world(), hull) && bound(&rig, escort),
        "inside the five minutes the seat is still the pilot's: {:?}",
        away_of(rig.tw.world(), hull)
    );
    // Five minutes: the unit passes to its mind.
    pass(&mut rig, &mut rx, &mut lines, 25);
    assert!(
        minded(rig.tw.world(), hull) && !bound(&rig, escort),
        "at five minutes the mind has the unit: {:?}",
        away_of(rig.tw.world(), hull)
    );
    // It drives to the ramp, is let out, and the host puts it aboard.
    let out = until(&mut rig, &mut rx, &mut lines, 180, |rig, _| {
        !in_field(rig, hull) && site_record(rig).is_some_and(|record| record.exited(hull))
    })
    .await;
    assert!(
        out,
        "the mind takes the escort out at its carrier's ramp: pose {:?}, away {:?}",
        mush_sim::pose_of(rig.tw.world(), hull).map(|(_, pose)| {
            tacsim::surface::excursion::site_cell(pose.x_cm, pose.y_cm)
        }),
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
    assert!(
        home,
        "the host puts the escort aboard its carrier: {}",
        run(&mut rig.tw, escort, "+tv/1")
    );
    // The black box, caught up: not one round of its own since the link was lost, with the
    // intruder its party holds Hostile standing ten cells off the whole way.
    let _ = until(&mut rig, &mut rx, &mut lines, 5, |_, _| false).await;
    assert_eq!(
        fired(&lines, hull),
        before,
        "the mind never fired: {} round(s) at the intruder",
        fired_at(&lines, hull, hostile)
    );
}

/// **Story 2** (this module's head): a clean QUIT hands over at once; the mind fires on nobody
/// it holds Hostile, and answers the unit that fires on it and no other.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mw7_an_absent_pilots_mind_answers_only_the_unit_that_fired_on_it() {
    let mut yard = yard().await;
    let (pilot, ward, raider, escort) = (yard.pilot, yard.ward, yard.raider, yard.escort);
    let free = run(&mut yard.rig.tw, pilot, "FIRE FREE");
    assert!(
        yard.wait(20, |rig, _| frame(rig, pilot)
            .is_some_and(|frame| frame.weapons_free))
            .await,
        "the ward is weapons free: {free}"
    );
    // A clean QUIT: no five minutes (Q-44-46).
    quit(&mut yard.rig, pilot);
    assert_eq!(
        away_of(yard.rig.tw.world(), ward),
        Some(Away {
            why: Why::Quit,
            since_wall_s: yard.rig.tw.world().now(),
            handed: true,
        }),
        "a quit hands the unit to its mind at once"
    );
    assert!(!bound(&yard.rig, pilot));
    // Twelve seconds with the raider marked Hostile eight cells off: nothing.
    let _ = yard.wait(12, |_, _| false).await;
    assert_eq!(
        fired(&yard.lines, ward),
        0,
        "the mind opens fire on nobody: {}",
        yard.exchange()
    );
    // The raider fires on it: it answers the raider.
    let answered = yard
        .raid(120, |_, lines| fired_at(lines, ward, raider) > 0)
        .await;
    yard.cease();
    assert!(answered, "the mind answers the unit that fired on it: {}", yard.exchange());
    let first_arrival = yard.lines.iter().position(|line| {
        matches!(&line.kind, EventKind::Impact { unit, target, .. } if *unit == raider && *target == ward)
    });
    let first_answer = yard
        .lines
        .iter()
        .position(|line| matches!(&line.kind, EventKind::Fired { unit, .. } if *unit == ward));
    assert!(
        first_arrival.is_some() && first_arrival < first_answer,
        "its first round follows the raider's first arrival: {first_arrival:?} then {first_answer:?}"
    );
    assert_eq!(
        fired_at(&yard.lines, ward, escort),
        0,
        "and it answers nobody else"
    );
    // No carrier of its party stands on the site: it holds where it stands (Q-44-42).
    assert!(
        in_field(&yard.rig, ward) && minded(yard.rig.tw.world(), ward),
        "{}",
        yard.exchange()
    );
}

/// **Story 3** (this module's head): a hold the pilot left holds through an attack.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mw7_a_hold_fire_left_by_the_pilot_holds_through_an_attack() {
    let mut yard = yard().await;
    let (pilot, ward, raider) = (yard.pilot, yard.ward, yard.raider);
    // Every IC seat opens at weapons hold (Q-CF-41), and this pilot leaves it there.
    assert!(
        frame(&yard.rig, pilot).is_some_and(|frame| !frame.weapons_free),
        "the seat is at weapons hold"
    );
    quit(&mut yard.rig, pilot);
    assert!(minded(yard.rig.tw.world(), ward));
    // The raider's fire arrives on it, round after round.
    let struck = yard
        .raid(120, |_, lines| arrived(lines, raider, ward) >= 3)
        .await;
    assert!(struck, "the raider's fire arrives on the ward: {}", yard.exchange());
    let _ = yard.wait(10, |_, _| false).await;
    yard.cease();
    assert_eq!(
        fired(&yard.lines, ward),
        0,
        "the hold its pilot left holds through the attack: {}",
        yard.exchange()
    );
}

/// **Story 4** (this module's head): ten minutes later the pilot is back in the seat, and their
/// first command wins.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mw7_a_pilot_who_reconnects_ten_minutes_later_is_back_in_the_seat() {
    let mut yard = yard().await;
    let (pilot, ward) = (yard.pilot, yard.ward);
    lose_link(&mut yard.rig, pilot);
    yard.pass(FIELD_LINKDEAD_GRACE_WALL_S - 1);
    assert!(
        !minded(yard.rig.tw.world(), ward) && bound(&yard.rig, pilot),
        "a second short of five minutes the seat is the pilot's: {:?}",
        away_of(yard.rig.tw.world(), ward)
    );
    // The server's own grace ends the session; the field's clock hands the unit over.
    end_session(&mut yard.rig, pilot);
    yard.pass(5);
    assert!(
        minded(yard.rig.tw.world(), ward) && !bound(&yard.rig, pilot),
        "at five minutes the mind has the unit: {:?}",
        away_of(yard.rig.tw.world(), ward)
    );
    // Ten minutes after the link was lost.
    yard.pass(FIELD_LINKDEAD_GRACE_WALL_S);
    assert!(minded(yard.rig.tw.world(), ward) && in_field(&yard.rig, ward));
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
    // Their first command is what the hull does.
    let focus = run(&mut yard.rig.tw, pilot, &format!("+focus #{}/pilot", ward.0));
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

/// **Story 5** (this module's head): a pilot knocked out while connected stops steering.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mw7_a_pilot_knocked_out_while_connected_stops_steering() {
    use tacsim::field::crew::{CrewCondition, crew_of};
    let mut rig = stocked_tuned(quiet);
    let (pilot, _, _) = on_pad_as(&mut rig, "Pilot", 1, "Salt Moth", 0);
    // A walker its pilot sits inside, the cockpit already shot away when the field opens.
    let ward = crate::field_crash::hull_of(
        &mut rig,
        "tacsim.unit.biped_mk1",
        "Ward Dog",
        WARD_AT,
        180_000,
    );
    seat(&mut rig, ward, pilot);
    crate::driving_rig::staff_hand(&mut rig, |ctx| {
        let _ = ctx.w.move_to(pilot, ward);
    });
    crate::field_crash::control_destroyed(&mut rig, ward);
    let mut rig = {
        let (captain, _, _) = on_pad_as(&mut rig, "Other", 2, "Tally Moth", 0);
        let (stranger, _, _) = on_pad_as(&mut rig, "Third", 3, "Lone Moth", 0);
        let escort = armed(&mut rig, "Red Hound", ESCORT_AT, 90_000);
        let raider = armed(&mut rig, "Grey Hound", RAIDER_AT, 0);
        seat(&mut rig, escort, captain);
        seat(&mut rig, raider, stranger);
        crate::driving_rig::warm(&mut rig);
        let _ = rig.tw.connect(pilot);
        for _ in 0..3 {
            let _ = second(&mut rig);
        }
        let (out, queued) = command(&mut rig, captain, "+field/fire E");
        assert_eq!(queued.len(), 1, "one promotion: {out}");
        land(&mut rig, &queued[0]).expect("the promotion lands");
        rig
    };
    let mut rx = egress(&mut rig);
    let mut lines = Vec::new();
    let down = until(&mut rig, &mut rx, &mut lines, 30, |rig, _| {
        crew_of(rig.tw.world(), pilot)
            .is_some_and(|crew| crew.condition == CrewCondition::Incapacitated)
    })
    .await;
    assert!(down, "the pilot is incapacitated in the hull");
    // Connected, and not at the controls: the mind has them, at once.
    assert!(rig.tw.world().is_connected(pilot));
    assert_eq!(
        away_of(rig.tw.world(), ward).map(|away| (away.why, away.handed)),
        Some((Why::Incapacitated, true)),
        "a pilot who is down loses the controls with no five minutes"
    );
    assert!(!bound(&rig, pilot), "the seat is released");
    let drive = run(&mut rig.tw, pilot, "DRIVE 150,110");
    let _ = until(&mut rig, &mut rx, &mut lines, 5, |_, _| false).await;
    assert!(
        !bound(&rig, pilot) && minded(rig.tw.world(), ward),
        "their DRIVE steers nothing: {drive}"
    );
    // Nor does a reconnect give the controls back to a pilot who is not fit (Q-44-43).
    lose_link(&mut rig, pilot);
    end_session(&mut rig, pilot);
    let heard = connect(&mut rig, pilot);
    assert!(
        heard.contains("You are in no state to fly"),
        "the returning pilot is told why: {heard}"
    );
    assert!(!bound(&rig, pilot) && minded(rig.tw.world(), ward));
}

/// **Story 6** (this module's head): a restart while the mind holds the unit resumes the mind,
/// seats nobody who is not connected, and the pilot who connects afterwards is seated.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mw7_a_restart_while_the_mind_holds_the_unit_resumes_the_mind() {
    let mut yard = yard().await;
    let (pilot, ward, raider, escort) = (yard.pilot, yard.ward, yard.raider, yard.escort);
    let (captain, stranger) = (yard.captain, yard.stranger);
    let _ = run(&mut yard.rig.tw, pilot, "FIRE FREE");
    assert!(
        yard.wait(20, |rig, _| frame(rig, pilot)
            .is_some_and(|frame| frame.weapons_free))
            .await
    );
    quit(&mut yard.rig, pilot);
    assert!(minded(yard.rig.tw.world(), ward));
    // The field saves with the mind holding the unit, then the server goes down and comes up.
    let _ = yard.wait(3, |_, _| false).await;
    let Yard {
        rig, mut rx, lines, ..
    } = yard;
    let shots_before = fired(&lines, ward);
    assert_eq!(shots_before, 0, "nothing fired before the restart");
    let mut rig = crash(rig, &mut rx, &[captain, stranger]).await;
    let mut rx = egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();
    assert!(
        until(&mut rig, &mut rx, &mut lines, 30, |rig, _| task(rig)
            .is_some())
        .await,
        "the field resumes"
    );
    assert!(
        minded(rig.tw.world(), ward) && !bound(&rig, pilot),
        "the boot seats nobody who is not there, and the mind still has the unit: {:?}",
        away_of(rig.tw.world(), ward)
    );
    let mut yard = Yard {
        rig,
        pilot,
        ward,
        stranger,
        raider,
        captain,
        escort,
        rx,
        lines,
    };
    // Still the absent player's rule: nothing until the raider's fire arrives, then an answer.
    let _ = yard.wait(12, |_, _| false).await;
    assert_eq!(
        fired(&yard.lines, ward),
        0,
        "the resumed mind opens fire on nobody: {}",
        yard.exchange()
    );
    let answered = yard
        .raid(120, |_, lines| fired_at(lines, ward, raider) > 0)
        .await;
    yard.cease();
    assert!(
        answered,
        "the resumed mind answers the unit that fired on it: {}",
        yard.exchange()
    );
    // The pilot connects and is seated.
    let heard = connect(&mut yard.rig, pilot);
    assert!(
        heard.contains("You are back at the controls"),
        "the returning pilot is told: {heard}"
    );
    assert!(
        away_of(yard.rig.tw.world(), ward).is_none() && bound(&yard.rig, pilot),
        "the pilot is seated again after the restart"
    );
}

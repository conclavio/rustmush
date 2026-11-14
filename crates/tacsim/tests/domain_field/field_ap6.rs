//! **Doc 45 unit AP.6 — the field's mind flies the posture** (Q-45-5 to Q-45-9, Q-45-12,
//! Q-45-16), over the composed world (`driving`'s keyed rig on Ahuna Flank) with the arena's
//! egress pumped by hand.
//!
//! Every story is a field of three parties or more, and every command is typed by a player who
//! is not staff. The ground of stories 1 to 4 is `escort_lease`'s with one more rig: a lessee
//! who sits in the cab of his armed rig, with a prospector of his own launched and standing
//! three cells off, nobody in its cab; a miner's prospector at work beside them; and an
//! intruder's armed hull ten cells to the east, whose player types every shot. The lessee
//! sets his rig's posture from its cab on open ground, marks the intruder's flight Hostile
//! (which opens the field without a round fired) and quits, so his rig is its mind's.
//!
//! 1. [`ap6_a_guard_answers_for_its_owners_other_rig_and_holds_fire_on_a_strangers_quarrel`]:
//!    on `GUARD` the rig fires on nobody with the marked intruder in range; still on nobody
//!    while the intruder's fire lands on the miner's rig beside it; and returns the intruder's
//!    fire once it lands on the lessee's other rig. It stays where it stood. What the intruder
//!    reads of it is what it read before.
//! 2. [`ap6_the_same_rig_on_hold_does_not_answer_for_its_neighbour`]: on `HOLD` the same fire
//!    on the lessee's other rig is not answered; fire on the rig itself is, from a seat that was
//!    never set to `FIRE FREE`. It is not taken home, and its player reads why at login.
//! 3. [`co2_the_same_rig_on_return_answers_on_its_way_home`] (doc 47 unit CO.2, R-47-3): on
//!    `RETURN`, fired on as it makes for its ramp, it answers the unit that fired, and nobody
//!    else, while it drives on; and it is put aboard.
//!    (Until CO.2: `ap6_the_same_rig_on_return_goes_home`, fired on only through its
//!    neighbour.)
//! 4. [`ap6_a_guard_that_loses_its_last_fire_channel_goes_home`]: a guard with two rounds
//!    left answers with them, and with nothing left to fire goes home; the lessee's word is
//!    still `GUARD`.
//! 5. [`ap6_a_party_declared_hostile_by_stance_is_not_fired_on_first_under_any_posture`]:
//!    three parties of one hull each, the ward's co-op having declared the raider's Hostile
//!    before the field opened. Under each posture in turn the ward's pilot quits, and the ward
//!    fires nothing with the raider eight cells off.

use mush_core::{GOD, ObjId};
use tacsim::arena::ledger::{EventKind, LedgerEvent};
use tacsim::arena_task::ArenaOut;
use tacsim::field::absent::{TAKEN_ON, minded};
use tacsim::posture::Posture;
use tacsim::posture::order::{POSTURE_SET, line};
use tokio::sync::mpsc::Receiver;

use crate::driving_rig::{
    Rig, cid, command, land, on_pad_as, run, staff_hand, stocked_tuned, to_ic,
};
use crate::escort_lease::{contact_near, fired, in_field, stand_up_out};
use crate::field_host_rig::{
    AHUNA, armed, egress, frame, seat, second, site_record, task, until, weapon_of,
};
use crate::field_mw7::{
    ESCORT_AT, RAIDER_AT, WARD_AT, arrived, bound, connect, fired_at, hostiles, quiet, quit,
};

/// Where the lessee's armed rig stands, his prospector, the miner's and the intruder.
const HULL_AT: (u16, u16) = (136, 122);
const KIN_AT: (u16, u16) = (139, 124);
const WORKER_AT: (u16, u16) = (140, 120);

/// The posture `unit`'s record holds.
fn held(rig: &Rig, unit: ObjId) -> Posture {
    tacsim::posture::record::of(rig.tw.world(), unit)
}

/// **What `viewer`'s cockpit holds of the unit plotted at `at`**: every word of the contact
/// row but its age.
fn seen(rig: &Rig, viewer: ObjId, at: (u16, u16)) -> Option<String> {
    contact_near(rig, viewer, at).map(|row| {
        format!(
            "{:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?}",
            row.designator,
            row.alias,
            row.class,
            row.relation,
            row.bearing_mdeg,
            row.range_cm,
            row.moving,
            row.cell,
            row.aspect,
            row.facing
        )
    })
}

/// The presence picture `who` reads off the site map: its lines that name a presence.
fn presence(rig: &mut Rig, who: ObjId) -> String {
    let map = run(&mut rig.tw, who, "+tv/map");
    let rows: Vec<&str> = map
        .lines()
        .filter(|line| line.contains("Presence"))
        .collect();
    assert!(!rows.is_empty(), "the site map names a presence: {map}");
    rows.join("\n")
}

/// **Leave `unit` one fire channel with `rounds` in it and every other magazine empty** (a
/// staff fixture standing in for a fight already had).
fn magazines(rig: &mut Rig, unit: ObjId, rounds: i64) {
    let keep = weapon_of(rig, unit);
    staff_hand(rig, |ctx| {
        let cells = mush_tactics::module::unit_of(ctx.w, unit)
            .expect("the unit")
            .clone();
        let ops: Vec<mush_tactics::UnitMutationOp> = cells
            .cells()
            .filter_map(|(addr, _)| match addr {
                mush_tactics::CellAddr::Component { component, cell }
                    if cell.as_str() == tacsim::definitions::unit::AMMO_CELL =>
                {
                    let kept = component.as_str() == keep
                        || component.as_str().ends_with(&format!(".{keep}"));
                    Some(mush_tactics::UnitMutationOp::SetCell {
                        addr: addr.clone(),
                        value: if kept { rounds } else { 0 },
                    })
                }
                _ => None,
            })
            .collect();
        assert!(!ops.is_empty(), "#{} carries a magazine", unit.0);
        let plan = mush_tactics::UnitMutationPlan::new(
            ops,
            None,
            Some(smol_str::SmolStr::new_static("test:ap6-magazines")),
        )
        .expect("a plan");
        mush_tactics::module::apply_plan(ctx, unit, &plan).expect("applied");
    });
}

/// **The ground of stories 1 to 4** (this module's head), in a running field, the lessee
/// connected and at the controls of his armed rig.
struct Ground {
    rig: Rig,
    /// The lessee and the armed rig he sits in.
    lessee: ObjId,
    hull: ObjId,
    /// His prospector, out on the ground with nobody in its cab.
    kin: ObjId,
    /// The miner's prospector: a stranger's.
    worker: ObjId,
    /// The intruder's player and hull.
    intruder: ObjId,
    hostile: ObjId,
    /// The miner, who reads the presence picture.
    miner: ObjId,
    /// The presence picture the miner read on the open surface, before and after the lessee
    /// set his rig's posture. (The presence picture is read from a rig on the ground, and the
    /// intruder's hull is a fixture; the miner is the other party with one out.)
    read: (String, String),
    rx: Receiver<ArenaOut>,
    lines: Vec<LedgerEvent>,
}

/// [`Ground`], the lessee's armed rig left on `posture`; `prepare` is a staff fixture run on
/// the open surface before anything else.
async fn ground(posture: Posture, prepare: impl FnOnce(&mut Rig, ObjId)) -> Ground {
    let mut rig = stocked_tuned(quiet);
    let (lessee, hull, intruder, hostile, others) = stand_up_out(
        &mut rig,
        1,
        Some((i64::from(KIN_AT.0), i64::from(KIN_AT.1))),
    );
    let kin = others[0];
    // The miner's prospector, and the miner: the rig standing where `escort_lease` put it.
    let worker = rig
        .tw
        .world()
        .module_state::<mush_tactics::module::UnitStore>()
        .expect("the unit store")
        .units
        .keys()
        .copied()
        .find(|unit| {
            mush_sim::pose_of(rig.tw.world(), *unit).is_some_and(|(_, pose)| {
                tacsim::surface::excursion::site_cell(pose.x_cm, pose.y_cm)
                    == (i64::from(WORKER_AT.0), i64::from(WORKER_AT.1))
            })
        })
        .expect("the miner's prospector stands on the ground");
    let miner = {
        let h = crate::driving_rig::handles(&rig.tw);
        crate::driving_rig::typed(
            &mut rig.tw,
            &h.families.excursions,
            &tacsim::surface::excursion::RigKey(worker),
        )
        .and_then(|record| record.excursion.map(|out| out.operator))
        .expect("the miner launched it")
    };
    prepare(&mut rig, hull);
    // Doc 45 unit AP.7: the miner's prospector has nobody in it either, and would go home when
    // the field opens. It is to stand beside the lessee's rigs, so the miner leaves it on HOLD
    // (the lessee's own prospector is left on HOLD by `stand_up_out`).
    crate::field_host_rig::left_on_hold(&mut rig, miner, 1);
    let _ = rig.tw.connect(lessee);
    for _ in 0..3 {
        let _ = second(&mut rig);
    }
    // The posture, from the rig's own cab on open ground (doc 45 unit AP.4).
    let before = presence(&mut rig, miner);
    if !posture.is_return() {
        assert_eq!(
            run(
                &mut rig.tw,
                lessee,
                &format!("+tv/1 posture {}", posture.as_str())
            ),
            line(POSTURE_SET, "Rig 1", posture),
            "the set says what was set, and no more"
        );
    }
    assert_eq!(held(&rig, hull), posture);
    for _ in 0..3 {
        let _ = second(&mut rig);
    }
    let after = presence(&mut rig, miner);
    let mut rx = egress(&mut rig);
    let mut lines = Vec::new();
    // The lessee marks the flight to his east Hostile: the site promotes, and nobody has fired.
    let (out, queued) = command(&mut rig, lessee, "+field/mark E=hostile");
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    land(&mut rig, &queued[0]).expect("the promotion lands");
    assert!(
        until(&mut rig, &mut rx, &mut lines, 15, |rig, _| task(rig)
            .is_some())
        .await,
        "the field's task opens"
    );
    let live = tacsim::field::live_on(rig.tw.world(), &cid(AHUNA)).expect("the live record");
    let parties: std::collections::BTreeSet<_> = [hull, kin, worker, hostile]
        .into_iter()
        .filter_map(|unit| live.party_of(unit))
        .collect();
    assert_eq!(
        parties.len(),
        4,
        "the lessee's two rigs, the miner's and the intruder's are four parties"
    );
    // His mark holds in the field: his own frame holds the intruder Hostile.
    let letter = live
        .party_of(hostile)
        .expect("the intruder's party")
        .letter();
    let marked = run(
        &mut rig.tw,
        lessee,
        &format!("+field/mark {letter}=hostile"),
    );
    assert!(
        until(&mut rig, &mut rx, &mut lines, 30, |rig, _| {
            hostiles(rig, lessee) > 0 && seen(rig, intruder, HULL_AT).is_some()
        })
        .await,
        "the lessee's frame holds the intruder Hostile, and the intruder plots his rig: {marked}"
    );
    assert!(
        frame(&rig, lessee).is_some_and(|frame| !frame.weapons_free),
        "the seat is at FIRE HOLD, where every seat in a field starts, and stays there"
    );
    Ground {
        rig,
        lessee,
        hull,
        kin,
        worker,
        intruder,
        hostile,
        miner,
        read: (before, after),
        rx,
        lines,
    }
}

impl Ground {
    /// Pump and tick until `done` holds or `secs` of wall time pass.
    async fn wait(
        &mut self,
        secs: u64,
        done: impl FnMut(&mut Rig, &[LedgerEvent]) -> bool,
    ) -> bool {
        until(&mut self.rig, &mut self.rx, &mut self.lines, secs, done).await
    }

    /// **The intruder's player keeps a lock on the unit plotted at `at` and fires** until
    /// `done` holds or `secs` of wall time pass: the two lines a player types, once a second.
    async fn raid(
        &mut self,
        at: (u16, u16),
        secs: u64,
        mut done: impl FnMut(&Rig, &[LedgerEvent]) -> bool,
    ) -> bool {
        let gun = weapon_of(&self.rig, self.hostile);
        for _ in 0..secs {
            if let Some(contact) = contact_near(&self.rig, self.intruder, at)
                && frame(&self.rig, self.intruder)
                    .is_some_and(|frame| frame.lock != Some(contact.id))
            {
                let _ = run(
                    &mut self.rig.tw,
                    self.intruder,
                    &format!("LOCK {}", contact.designator),
                );
            }
            let _ = run(&mut self.rig.tw, self.intruder, &format!("FIRE {gun}"));
            if self.wait(1, |rig, lines| done(rig, lines)).await {
                return true;
            }
        }
        false
    }

    /// The intruder's player ceases fire.
    fn cease(&mut self) {
        let gun = weapon_of(&self.rig, self.hostile);
        let _ = run(&mut self.rig.tw, self.intruder, &format!("FIRE {gun} OFF"));
    }

    /// The lessee quits: his rig is its mind's at once (Q-44-46).
    fn leave(&mut self) {
        quit(&mut self.rig, self.lessee);
        assert!(
            minded(self.rig.tw.world(), self.hull) && !bound(&self.rig, self.lessee),
            "a quit hands the rig to its mind"
        );
    }

    /// What the black box holds, for a failed wait.
    fn exchange(&self) -> String {
        format!(
            "the rig fired {} (at the intruder {}); the intruder fired {}, arrived on the rig {}, on its kin {}, on the miner's {}",
            fired(&self.lines, self.hull),
            fired_at(&self.lines, self.hull, self.hostile),
            fired(&self.lines, self.hostile),
            arrived(&self.lines, self.hostile, self.hull),
            arrived(&self.lines, self.hostile, self.kin),
            arrived(&self.lines, self.hostile, self.worker),
        )
    }

    /// **The rig is taken out at its carrier's ramp and put aboard**, within `secs`.
    async fn goes_home(&mut self, secs: u64) -> bool {
        let hull = self.hull;
        let out = self
            .wait(secs, |rig, _| {
                !in_field(rig, hull) && site_record(rig).is_some_and(|record| record.exited(hull))
            })
            .await;
        if !out {
            return false;
        }
        self.wait(120, |rig, _| {
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
        .await
    }

    /// Whether the rig stands in the field where it stood, as the intruder plots it.
    fn stays(&mut self) -> bool {
        let hull = self.hull;
        in_field(&self.rig, hull)
            && minded(self.rig.tw.world(), hull)
            && site_record(&mut self.rig).is_none_or(|record| !record.exited(hull))
            && contact_near(&self.rig, self.intruder, HULL_AT)
                .is_some_and(|row| row.cell == Some(HULL_AT))
    }
}

/// The first line of `lines` that `pick` takes.
fn first(lines: &[LedgerEvent], pick: impl Fn(&EventKind) -> bool) -> Option<usize> {
    lines.iter().position(|line| pick(&line.kind))
}

/// **Story 1** (this module's head): `GUARD` answers for its owner's other rig, and holds fire
/// on a stranger's quarrel beside it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn ap6_a_guard_answers_for_its_owners_other_rig_and_holds_fire_on_a_strangers_quarrel() {
    let mut g = ground(Posture::Guard, |_, _| {}).await;
    let (hull, kin, worker, hostile) = (g.hull, g.kin, g.worker, g.hostile);
    assert_eq!(
        g.read.0, g.read.1,
        "the presence picture an attacker reads does not move with a posture"
    );
    let plotted = seen(&g.rig, g.intruder, HULL_AT);
    g.leave();
    // Twelve seconds with the marked intruder ten cells off: nothing, and the rig is where it
    // was. The intruder's own row of it reads as it did when its pilot flew it.
    let _ = g.wait(12, |_, _| false).await;
    assert_eq!(
        fired(&g.lines, hull),
        0,
        "a party only marked Hostile is not fired on: {}",
        g.exchange()
    );
    assert!(g.stays(), "a guard stays where it stood");
    assert_eq!(
        seen(&g.rig, g.intruder, HULL_AT),
        plotted,
        "what an attacker reads of the rig is what it read before its mind took it"
    );
    let miner = g.miner;
    assert_eq!(
        presence(&mut g.rig, miner),
        g.read.0,
        "and the presence picture is what it was with the rig's pilot in its seat on RETURN"
    );
    // A stranger's quarrel: the intruder's fire lands on the miner's rig beside it.
    let struck = g
        .raid(WORKER_AT, 120, |_, lines| {
            arrived(lines, hostile, worker) >= 2
        })
        .await;
    assert!(
        struck,
        "the intruder's fire arrives on the miner's rig: {}",
        g.exchange()
    );
    let _ = g.wait(10, |_, _| false).await;
    assert_eq!(
        fired(&g.lines, hull),
        0,
        "a guard holds its fire on a stranger's quarrel: {}",
        g.exchange()
    );
    // Its owner's other rig is fired on: it answers the unit that fired.
    let answered = g
        .raid(KIN_AT, 150, |_, lines| fired_at(lines, hull, hostile) > 0)
        .await;
    g.cease();
    assert!(
        answered,
        "a guard returns fire on the unit that hit its owner's other rig: {}",
        g.exchange()
    );
    let on_kin = first(
        &g.lines,
        |kind| matches!(kind, EventKind::Impact { unit, target, .. } if *unit == hostile && *target == kin),
    );
    let answer = first(
        &g.lines,
        |kind| matches!(kind, EventKind::Fired { unit, .. } if *unit == hull),
    );
    assert!(
        on_kin.is_some() && on_kin < answer,
        "its first round follows the first arrival on its owner's rig: {on_kin:?} then {answer:?}"
    );
    assert_eq!(
        arrived(&g.lines, hostile, hull),
        0,
        "and the intruder never fired on the guard itself: {}",
        g.exchange()
    );
    assert_eq!(
        fired(&g.lines, hull),
        fired_at(&g.lines, hull, hostile),
        "it fires on the unit that fired, and on nobody else"
    );
    assert!(
        in_field(&g.rig, hull) && minded(g.rig.tw.world(), hull),
        "it is still in the field, its mind's"
    );
    assert_eq!(held(&g.rig, hull), Posture::Guard);
}

/// **Story 2** (this module's head): `HOLD` does not answer for its neighbour, answers for
/// itself, and stays.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ap6_the_same_rig_on_hold_does_not_answer_for_its_neighbour() {
    let mut g = ground(Posture::Hold, |_, _| {}).await;
    let (lessee, hull, kin, hostile) = (g.lessee, g.hull, g.kin, g.hostile);
    g.leave();
    // The fire story 1's guard answered: on its owner's other rig.
    let struck = g
        .raid(KIN_AT, 150, |_, lines| arrived(lines, hostile, kin) >= 3)
        .await;
    assert!(
        struck,
        "the intruder's fire arrives on the lessee's other rig: {}",
        g.exchange()
    );
    let _ = g.wait(12, |_, _| false).await;
    assert_eq!(
        fired(&g.lines, hull),
        0,
        "a rig on HOLD does not answer for its neighbour: {}",
        g.exchange()
    );
    assert!(g.stays(), "a rig on HOLD stays where it stood");
    // Fire on the rig itself is returned, from a seat nobody ever set to FIRE FREE.
    let answered = g
        .raid(HULL_AT, 150, |_, lines| fired_at(lines, hull, hostile) > 0)
        .await;
    g.cease();
    assert!(
        answered,
        "a rig on HOLD returns fire on the unit that fired on it: {}",
        g.exchange()
    );
    let on_it = first(
        &g.lines,
        |kind| matches!(kind, EventKind::Impact { unit, target, .. } if *unit == hostile && *target == hull),
    );
    let answer = first(
        &g.lines,
        |kind| matches!(kind, EventKind::Fired { unit, .. } if *unit == hull),
    );
    assert!(
        on_it.is_some() && on_it < answer,
        "its first round follows the first arrival on it: {on_it:?} then {answer:?}"
    );
    assert_eq!(
        fired(&g.lines, hull),
        fired_at(&g.lines, hull, hostile),
        "and on nobody else"
    );
    assert!(
        in_field(&g.rig, hull) && site_record(&mut g.rig).is_none_or(|record| !record.exited(hull)),
        "it is not taken home"
    );
    // Its player reads at login what its mind was left to do.
    let heard = connect(&mut g.rig, lessee);
    let told = TAKEN_ON
        .replace("{unit}", &hull.0.to_string())
        .replace("{posture}", "HOLD");
    assert!(heard.contains(&told), "{heard}");
}

/// `unit`'s positions in the field's position record, centimetres, in the order recorded:
/// each with the index of the line that carries it.
fn track(lines: &[LedgerEvent], of: ObjId) -> Vec<(usize, i64, i64)> {
    lines
        .iter()
        .enumerate()
        .filter_map(|(at, line)| match &line.kind {
            EventKind::PositionSummary { units } => units
                .iter()
                .find(|(unit, ..)| *unit == of)
                .map(|(_, x, y, ..)| (at, *x, *y)),
            _ => None,
        })
        .collect()
}

/// The cell `unit` was last recorded on, if the record has it.
fn last_cell(lines: &[LedgerEvent], of: ObjId) -> Option<(u16, u16)> {
    track(lines, of).last().map(|(_, x, y)| {
        let (cx, cy) = tacsim::surface::excursion::site_cell(*x, *y);
        (
            u16::try_from(cx).unwrap_or(0),
            u16::try_from(cy).unwrap_or(0),
        )
    })
}

/// One position record of a unit: the index of its line, and its `x` and `y`, centimetres.
type Recorded = (usize, i64, i64);

/// **Where the field recorded `hull` while it answered**: the first position record after
/// its first round, and the first record after a round it fired once that record had passed
/// (the field records every unit every five arena seconds). `None` until it has fired across
/// two records.
fn answered_across(lines: &[LedgerEvent], hull: ObjId) -> Option<(Recorded, Recorded)> {
    let shots: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| matches!(&line.kind, EventKind::Fired { unit, .. } if *unit == hull))
        .map(|(at, _)| at)
        .collect();
    let path = track(lines, hull);
    let first_shot = *shots.first()?;
    let after = path.iter().position(|(at, ..)| *at >= first_shot)?;
    let from = *path.get(after)?;
    let later = *shots.iter().find(|at| **at > from.0)?;
    let to = *path.iter().find(|(at, ..)| *at >= later)?;
    Some((from, to))
}

impl Ground {
    /// **The intruder's player keeps a lock on the lessee's rig wherever the field last
    /// recorded it and fires** until `done` holds or `secs` of wall time pass.
    async fn raid_the_rig(
        &mut self,
        secs: u64,
        mut done: impl FnMut(&Rig, &[LedgerEvent]) -> bool,
    ) -> bool {
        let gun = weapon_of(&self.rig, self.hostile);
        for _ in 0..secs {
            // Where the field last recorded it (every five arena seconds), and the contact
            // the intruder plots nearest there: not one of the two rigs left on HOLD.
            let at = last_cell(&self.lines, self.hull).unwrap_or(HULL_AT);
            let apart = |a: (u16, u16), b: (u16, u16)| a.0.abs_diff(b.0).max(a.1.abs_diff(b.1));
            let target = frame(&self.rig, self.intruder).and_then(|frame| {
                frame
                    .contacts
                    .iter()
                    .filter_map(|contact| contact.cell.map(|cell| (cell, contact)))
                    .filter(|(cell, _)| apart(*cell, KIN_AT) > 1 && apart(*cell, WORKER_AT) > 1)
                    .min_by_key(|(cell, _)| apart(*cell, at))
                    .filter(|(cell, _)| apart(*cell, at) <= 12)
                    .map(|(_, contact)| contact.clone())
            });
            if let Some(contact) = target
                && frame(&self.rig, self.intruder)
                    .is_some_and(|frame| frame.lock != Some(contact.id))
            {
                let _ = run(
                    &mut self.rig.tw,
                    self.intruder,
                    &format!("LOCK {}", contact.designator),
                );
            }
            let _ = run(&mut self.rig.tw, self.intruder, &format!("FIRE {gun}"));
            if self.wait(1, |rig, lines| done(rig, lines)).await {
                return true;
            }
        }
        false
    }
}

/// **Story 3** (this module's head): `RETURN` answers the unit that fires on it on its way
/// home, and nobody else, and does not stop going home to do it (doc 47 unit CO.2,
/// R-47-3). Until CO.2 this story was `ap6_the_same_rig_on_return_goes_home`: the rig was
/// fired on only through its neighbour, and fired nothing; under fire itself the doctrine's
/// cornered stand would have stopped it where it was.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn co2_the_same_rig_on_return_answers_on_its_way_home() {
    let mut g = ground(Posture::Return, |_, _| {}).await;
    let (hull, kin, hostile) = (g.hull, g.kin, g.hostile);
    g.leave();
    // The intruder fires on the rig itself as it makes for its ramp, wherever it has got
    // to: it answers, more than once, and nobody else. (That it does not answer for its
    // neighbour on RETURN is the gate's: `arena::weapons::tests`, AP.6's guard test.)
    let answered = g
        .raid_the_rig(150, |_, lines| answered_across(lines, hull).is_some())
        .await;
    g.cease();
    assert!(
        answered,
        "a rig on RETURN answers the unit that fires on it: {}; last recorded at {:?}",
        g.exchange(),
        last_cell(&g.lines, hull)
    );
    assert_eq!(
        arrived(&g.lines, hostile, kin),
        0,
        "the story's premise: the intruder fired on the rig, not its neighbour"
    );
    let on_it = first(
        &g.lines,
        |kind| matches!(kind, EventKind::Impact { unit, target, .. } if *unit == hostile && *target == hull),
    );
    let answer = first(
        &g.lines,
        |kind| matches!(kind, EventKind::Fired { unit, .. } if *unit == hull),
    );
    assert!(
        on_it.is_some() && on_it < answer,
        "its first round follows the first arrival on it: {on_it:?} then {answer:?}"
    );
    assert_eq!(
        fired(&g.lines, hull),
        fired_at(&g.lines, hull, hostile),
        "and on nobody else"
    );
    // It kept going while it answered: where the field recorded it just after its first
    // round, and where it recorded it after a later round, differ.
    let (from, to) = answered_across(&g.lines, hull).expect("the raid ended on it");
    let moved_cm = (to.1 - from.1).abs().max((to.2 - from.2).abs());
    assert!(
        moved_cm >= 100,
        "it drove on while it answered: {from:?} then {to:?}"
    );
    // And it is home.
    assert!(
        g.goes_home(240).await,
        "a rig on RETURN is taken out at its carrier's ramp and put aboard: {}",
        g.exchange()
    );
}

/// **Story 4** (this module's head): a guard that loses its last fire channel goes home.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ap6_a_guard_that_loses_its_last_fire_channel_goes_home() {
    // One fire channel with two rounds in it, and nothing else to fire.
    let mut g = ground(Posture::Guard, |rig, hull| magazines(rig, hull, 2)).await;
    let (hull, hostile) = (g.hull, g.hostile);
    g.leave();
    let _ = g.wait(8, |_, _| false).await;
    assert!(g.stays(), "with a round left it stands its guard");
    // It answers for its owner's other rig with what it has.
    let spent = g
        .raid(KIN_AT, 180, |_, lines| fired_at(lines, hull, hostile) >= 2)
        .await;
    g.cease();
    assert!(
        spent,
        "the guard answers with its last two rounds: {}",
        g.exchange()
    );
    // Nothing left to fire: it can no longer defend itself, and goes home.
    assert!(
        g.goes_home(240).await,
        "a guard with no fire channel left is taken home: {}",
        g.exchange()
    );
    assert_eq!(
        fired(&g.lines, hull),
        2,
        "two rounds were all it had: {}",
        g.exchange()
    );
    assert_eq!(
        held(&g.rig, hull),
        Posture::Guard,
        "the lessee's word is kept: the rig guards again once it can"
    );
}

/// `who` joins `faction` openly as an officer.
fn officer(rig: &mut Rig, faction: &str, who: ObjId) {
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@faction/add {faction}=#{}/level:2", who.0),
    );
    assert!(
        !out.contains("No such"),
        "#{} joins {faction}: {out}",
        who.0
    );
}

/// **Story 5** (this module's head): a party declared Hostile by stance is not fired on first,
/// under any posture.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn ap6_a_party_declared_hostile_by_stance_is_not_fired_on_first_under_any_posture() {
    // A hull a mind flies under no rule falls back first and fights after the doctrine's rally
    // time (doc 45 cut AP.5's measurement); it is shortened so each quiet stretch is past it.
    let mut rig = stocked_tuned(|cfg| {
        quiet(cfg);
        cfg.arena.reflex.rally_ms = 4_000;
    });
    // Three parties of one hull each: the ward, a raider eight cells off, an escort.
    let (pilot, _, _) = on_pad_as(&mut rig, "Pilot", 1, "Salt Moth", 0);
    let (captain, _, _) = on_pad_as(&mut rig, "Other", 2, "Tally Moth", 0);
    let (stranger, _, _) = on_pad_as(&mut rig, "Third", 3, "Lone Moth", 0);
    let ward = armed(&mut rig, "Ward Dog", WARD_AT, 180_000);
    let escort = armed(&mut rig, "Red Hound", ESCORT_AT, 90_000);
    let raider = armed(&mut rig, "Grey Hound", RAIDER_AT, 0);
    seat(&mut rig, ward, pilot);
    seat(&mut rig, escort, captain);
    seat(&mut rig, raider, stranger);
    // The ward's co-op declares the raider's Hostile, by its officer's own command, before any
    // field is open (doc 40 cut FR.1): nobody in this field marks anybody.
    for name in ["Concord", "Basin"] {
        let out = run(&mut rig.tw, GOD, &format!("@faction/create {name}"));
        assert!(out.contains(name), "{out}");
    }
    officer(&mut rig, "Concord", pilot);
    officer(&mut rig, "Basin", stranger);
    let declared = run(&mut rig.tw, pilot, "+faction/stance Concord=Basin/hostile");
    assert!(declared.contains("Hostile"), "{declared}");
    crate::driving_rig::warm(&mut rig);
    let _ = rig.tw.connect(pilot);
    let mut rx = egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();
    for _ in 0..3 {
        let _ = second(&mut rig);
    }
    // The third captain's fire order opens the field.
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
    let (ours, theirs) = (
        live.party_of(ward).expect("the ward's party"),
        live.party_of(raider).expect("the raider's party"),
    );
    assert!(
        live.parties
            .iter()
            .any(|party| party.party == ours && party.stance_hostile.contains(&theirs)),
        "the declared stance is the ward's party's opening relation: {:?}",
        live.parties
    );
    assert!(
        until(&mut rig, &mut rx, &mut lines, 30, |rig, _| hostiles(
            rig, pilot
        ) > 0)
        .await,
        "the ward's frame holds the raider Hostile, by the stance alone"
    );
    // Its opening shot aside, the escort fires nothing; the ward has fired nothing at all.
    assert_eq!(fired(&lines, ward), 0);

    for posture in [Posture::Guard, Posture::Hold, Posture::Return] {
        let set = run(
            &mut rig.tw,
            pilot,
            &format!("MIND POSTURE {}", posture.label()),
        );
        assert_eq!(held(&rig, ward), posture, "{set}");
        assert!(
            until(&mut rig, &mut rx, &mut lines, 20, |rig, _| frame(
                rig, pilot
            )
            .is_some_and(
                |frame| frame.posture == posture && hostiles(rig, pilot) > 0
            ))
            .await,
            "the hull's frame reads {posture} with the raider held Hostile"
        );
        quit(&mut rig, pilot);
        assert!(
            minded(rig.tw.world(), ward) && !bound(&rig, pilot),
            "{posture}: a quit hands the ward to its mind"
        );
        let _ = until(&mut rig, &mut rx, &mut lines, 20, |_, _| false).await;
        assert_eq!(
            fired(&lines, ward),
            0,
            "{posture}: a party declared Hostile by stance is not fired on first"
        );
        assert!(
            in_field(&rig, ward),
            "{posture}: with no carrier of its party on the site it holds where it stands"
        );
        // Its pilot comes back and takes the cockpit again, for the next word.
        let heard = connect(&mut rig, pilot);
        assert!(heard.contains("You are back at the controls"), "{heard}");
        let focus = run(&mut rig.tw, pilot, &format!("+focus #{}/cockpit", ward.0));
        assert!(
            until(&mut rig, &mut rx, &mut lines, 30, |rig, _| frame(
                rig, pilot
            )
            .is_some())
            .await,
            "the returning pilot's cockpit frame arrives: {focus}"
        );
    }
}

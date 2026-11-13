//! **Doc 45 unit AP.7 — rigs on a plan, and carriers** (Q-45-1, Q-45-10, Q-45-11), over the
//! composed world (`driving`'s keyed rig on Ahuna Flank) with the arena's egress pumped by
//! hand.
//!
//! Every story is a field of four parties, and every command is typed by a player who is not
//! staff. The ground is one operator's whole outfit left to itself: his carrier on the pad,
//! his leased armed rig launched and stood off with nobody in its cab, and his prospector on
//! a pocket it has sounded, working a fill plan he filed with his own command. He has logged
//! off aboard his carrier. Beside them a second miner's prospector stands on `HOLD`, its
//! operator connected aboard a carrier of her own, and ten cells east sits an intruder's
//! armed hull, whose co-op the operator's co-op declared Hostile before any of this. The
//! second miner's mark on the intruder opens the field, so nobody of the operator's is there
//! to see it open.
//!
//! 1. [`ap7_an_unattended_rig_on_a_raided_site_goes_home_and_its_operator_reads_the_plans_end`]:
//!    both his rigs are their minds' from the turn the field takes them in. The armed one
//!    fires on nobody, with the declared intruder in range and the intruder's fire landing on
//!    the second miner's rig beside it. Both drive to the ramp and are put aboard. The
//!    prospector's plan ends there, and at login the operator reads that the field opened
//!    around each rig and that the plan ended, with why.
//! 2. [`ap7_the_same_rig_on_hold_is_still_there_at_demotion_and_its_plan_runs_on`]:
//!    the prospector, set to `HOLD` from aboard its carrier before he left, stands on its
//!    cell through the field. Its plan starts nothing while the field runs, and once the
//!    site demotes its next act is the plan's own.
//! 3. [`ap7_a_carrier_with_nobody_aboard_stays_on_its_pad_and_takes_its_rigs_aboard`]:
//!    the carrier is its mind's too: the field's own snapshot holds it as an absent
//!    player's unit standing on `HOLD`, though its record says `RETURN`. It takes the armed
//!    rig into its bay when that rig reaches the ramp, and a minute and a half on it is still
//!    on its pad, in the field, with nothing having asked it out. The prospector, set to
//!    `GUARD`, has nothing to fire: in that time, long enough for story 1's prospector to
//!    have gone home from the same cell, it is never let out.
//! 4. [`ap7_a_rig_whose_operator_sits_in_his_other_rig_is_still_put_aboard`] (found by doc
//!    45 unit AP.11's story; `escort_lease`'s ground): the operator is at the controls of his
//!    armed rig, and his prospector stands beside it on `RETURN` with nobody in its cab. Its
//!    mind takes it to the ramp, and the host puts it aboard as its operator, who is neither
//!    aboard the carrier nor in that rig.

use mush_core::{GOD, ObjId};
use tacsim::arena::ledger::LedgerEvent;
use tacsim::arena_task::ArenaOut;
use tacsim::field::absent::{UNATTENDED, UNATTENDED_ON, Why, away_of, minded};
use tacsim::port::tv::{END_FIELD, PLAN_ENDED};
use tacsim::posture::Posture;
use tacsim::posture::order::{POSTURE_SET, line};
use tacsim::surface::plan::{ActBy, FillState, PlanEnd};
use tacsim::surface::{ActKind, Word};
use tokio::sync::mpsc::Receiver;

use crate::driving_rig::{
    RAMP, RAMP_CELL, Rig, act_go, ahuna, cid, command, finish, ic_now, land, land_all, launched,
    on_ground_at, on_pad_as, on_pad_with, parent, record_of, run, stocked_tuned, to_cell, to_ic,
    warm,
};
use crate::escort_lease::{contact_near, fired, in_field, lease_escort, stand_up_out, top_up};
use crate::field_host_rig::{
    AHUNA, armed, egress, frame, seat, second, site_record, task, tune, until, weapon_of,
};
use crate::field_mw7::{arrived, connect, quiet, quit};

/// Where the operator's armed rig stands, the second miner's prospector and the intruder.
const ARMED_AT: (i64, i64) = (136, 122);
const WORKER_AT: (u16, u16) = (140, 120);
const INTRUDER_AT: (i64, i64) = (146, 122);

/// **The ground** (this module's head), in a running field.
struct Site {
    rig: Rig,
    /// The operator, logged off aboard his carrier, and the carrier.
    owner: ObjId,
    carrier: ObjId,
    /// His armed rig and his prospector, nobody in either.
    escort: ObjId,
    digger: ObjId,
    /// The cell the prospector stands on: the stand of the pocket its plan names.
    stand: (i64, i64),
    /// The second miner's prospector, on `HOLD`.
    worker: ObjId,
    /// The intruder's player and hull.
    intruder: ObjId,
    hostile: ObjId,
    rx: Receiver<ArenaOut>,
    lines: Vec<LedgerEvent>,
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

/// **A pocket near the armed rig that reads fair or better in metals**, from the keyed truth
/// a test may read and a player may not: its address and the stand at its middle. Never one
/// another unit of the story stands in.
fn pocket_to_work(rig: &mut Rig) -> (String, (i64, i64)) {
    let ground = ahuna(&mut rig.tw);
    let taken = [
        ARMED_AT,
        (i64::from(WORKER_AT.0), i64::from(WORKER_AT.1)),
        INTRUDER_AT,
        RAMP_CELL,
    ];
    let mut near: Vec<(i64, i64)> = (29..=34)
        .flat_map(|py| (33..=37).map(move |px| (px, py)))
        .collect();
    near.sort_by_key(|(px, py)| (px * 4 + 2 - 136).abs().max((py * 4 + 2 - 128).abs()));
    near.into_iter()
        .find_map(|(px, py)| {
            let stand = (px * 4 + 2, py * 4 + 2);
            // Well clear of the second miner's rig, so the intruder's lock on the unit
            // plotted there is on that rig and no other.
            let apart = |at: &(i64, i64)| (at.0 - stand.0).abs().max((at.1 - stand.1).abs());
            let clear = taken.iter().all(|at| apart(at) >= 2) && apart(&taken[1]) >= 5;
            let fair = ground
                .ground
                .geology
                .pocket(px, py)
                .is_some_and(|row| row.classes[0].code() >= Word::Fair.code());
            let footing = tacsim::surface::chart::footing_at(ground.surface(), stand).is_ok();
            (clear && fair && footing).then(|| {
                (
                    tacsim::surface::address::pocket_address(px, py).expect("an address"),
                    stand,
                )
            })
        })
        .expect("a fair metal pocket stands near the pad")
}

/// [`Site`], the operator's prospector left on `posture`, the field tuned by `tune_field`.
#[allow(clippy::too_many_lines)]
async fn site(posture: Posture, tune_field: fn(&mut tacsim::TacsimConfig)) -> Site {
    // A hull a mind flies under no rule falls back first and fights after the doctrine's
    // rally time (doc 45 cut AP.5's measurement): shortened, so a rig that would fire first
    // has done so well inside each story's quiet stretch.
    let mut rig = stocked_tuned(|cfg| {
        tune_field(cfg);
        cfg.arena.reflex.rally_ms = 4_000;
    });
    // The second miner's prospector, launched and stood off the ramp.
    let (miner, _, miner_tvs) = on_pad_as(&mut rig, "Miner", 2, "Tally Moth", 1);
    let worker = miner_tvs[0];
    launched(&mut rig, miner, worker);
    to_cell(
        &mut rig,
        worker,
        (i64::from(WORKER_AT.0), i64::from(WORKER_AT.1)),
    );
    // The operator's outfit: an armed rig in slot 1 and a prospector in slot 2.
    let (owner, row, tvs) = on_pad_with(&mut rig, "Pilot", 1, "Salt Moth", |rig, pilot| {
        lease_escort(rig, pilot);
        top_up(rig, pilot, 1_000_000);
        crate::driving_rig::lease_one(rig, pilot);
    });
    let carrier = ObjId(row.hull);
    let (escort, digger) = (tvs[0], tvs[1]);
    // Both go down the ramp by his own command from aboard the carrier: nobody boards.
    for (slot, unit) in [(2, digger), (1, escort)] {
        let launch = act_go(&mut rig, owner, &format!("+tv/{slot} launch go"));
        assert!(!launch.is_empty(), "{launch}");
        let _ = finish(&mut rig, unit, owner);
        assert!(
            on_ground_at(&rig.tw, unit, RAMP),
            "rig {slot} is down the ramp"
        );
        if unit == escort {
            to_cell(&mut rig, unit, ARMED_AT);
        }
    }
    // The intruder: a third player's armed hull standing to the east.
    let (intruder, _, _) = on_pad_as(&mut rig, "Intruder", 3, "Lone Moth", 0);
    let hostile = armed(&mut rig, "Grey Hound", INTRUDER_AT, 270_000);
    seat(&mut rig, hostile, intruder);
    // The operator's co-op declares the intruder's Hostile, by his own command (doc 40 cut
    // FR.1): his rigs hold the intruder Hostile from the field's opening, and he marks nobody.
    for name in ["Concord", "Basin"] {
        let out = run(&mut rig.tw, GOD, &format!("@faction/create {name}"));
        assert!(out.contains(name), "{out}");
    }
    officer(&mut rig, "Concord", owner);
    officer(&mut rig, "Basin", intruder);
    let declared = run(&mut rig.tw, owner, "+faction/stance Concord=Basin/hostile");
    assert!(declared.contains("Hostile"), "{declared}");
    warm(&mut rig);
    let _ = rig.tw.connect(owner);
    let _ = rig.tw.connect(miner);
    // The prospector sounds the pocket it stands on, and its operator files a plan to mine it.
    let (addr, stand) = pocket_to_work(&mut rig);
    to_cell(&mut rig, digger, stand);
    let sound = act_go(&mut rig, owner, "+tv/2 sound quiet go");
    assert!(!sound.is_empty(), "{sound}");
    let _ = finish(&mut rig, digger, owner);
    // The postures, each from aboard the unit's own carrier (doc 45 unit AP.4).
    if !posture.is_return() {
        assert_eq!(
            run(
                &mut rig.tw,
                owner,
                &format!("+tv/2 posture {}", posture.as_str())
            ),
            line(POSTURE_SET, "Rig 2", posture),
        );
    }
    assert_eq!(
        run(&mut rig.tw, miner, "+tv/1 posture hold"),
        line(POSTURE_SET, "Rig 1", Posture::Hold),
    );
    // The presence picture is published by the surface tick: it runs now, before the plan is
    // filed, so the plan has started nothing when the field opens.
    for _ in 0..3 {
        let _ = second(&mut rig);
    }
    let (out, queued) = command(&mut rig, owner, &format!("+tv/2 plan {addr} go"));
    assert_eq!(queued.len(), 1, "one filing: {out}");
    land_all(&mut rig, &queued);
    let filed = record_of(&mut rig.tw, digger)
        .and_then(|record| record.plan)
        .expect("the plan is filed");
    assert_eq!(
        (filed.author, filed.state, filed.cursor),
        (owner, FillState::Live, 0),
        "a live plan, nothing of it started: {out}"
    );
    // He logs off aboard his carrier. Nothing has ticked since the filing, so the plan has
    // started nothing when the field opens.
    quit(&mut rig, owner);
    assert!(!rig.tw.world().is_connected(owner));
    let mut rx = egress(&mut rig);
    let mut lines = Vec::new();
    // The second miner marks the flight to her east Hostile: the site promotes.
    let (out, queued) = command(&mut rig, miner, "+field/mark E=hostile");
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    land(&mut rig, &queued[0]).expect("the promotion lands");
    assert!(
        until(&mut rig, &mut rx, &mut lines, 15, |rig, _| task(rig)
            .is_some())
        .await,
        "the field's task opens"
    );
    let live = tacsim::field::live_on(rig.tw.world(), &cid(AHUNA)).expect("the live record");
    let parties: std::collections::BTreeSet<_> = [escort, digger, worker, hostile]
        .into_iter()
        .filter_map(|unit| live.party_of(unit))
        .collect();
    assert_eq!(
        parties.len(),
        4,
        "the operator's two rigs, the second miner's and the intruder's are four parties"
    );
    let theirs = live.party_of(hostile).expect("the intruder's party");
    for unit in [escort, digger] {
        let ours = live.party_of(unit).expect("the rig's party");
        assert!(
            live.parties
                .iter()
                .any(|party| party.party == ours && party.stance_hostile.contains(&theirs)),
            "the declared stance is #{}'s party's opening relation: {:?}",
            unit.0,
            live.parties
        );
    }
    Site {
        rig,
        owner,
        carrier,
        escort,
        digger,
        stand,
        worker,
        intruder,
        hostile,
        rx,
        lines,
    }
}

impl Site {
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
        let mut held = false;
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
            held = self.wait(1, |rig, lines| done(rig, lines)).await;
            if held {
                break;
            }
        }
        let _ = run(&mut self.rig.tw, self.intruder, &format!("FIRE {gun} OFF"));
        held
    }

    /// Whether `unit` is in the operator's carrier's bay.
    fn aboard(&self, unit: ObjId) -> bool {
        tacsim::surface::act::where_is(self.rig.tw.world(), unit)
            == (tacsim::surface::act::Where::Bay { hull: self.carrier })
    }

    /// Whether `unit` stands in the field on `cell`, as the intruder plots it.
    fn stands(&self, unit: ObjId, cell: (i64, i64)) -> bool {
        let at = (
            u16::try_from(cell.0).unwrap_or(0),
            u16::try_from(cell.1).unwrap_or(0),
        );
        in_field(&self.rig, unit)
            && contact_near(&self.rig, self.intruder, at).is_some_and(|row| row.cell == Some(at))
    }

    /// **`unit` is let out at its carrier's ramp and put aboard**, within `secs`: the kernel
    /// lets it go, the record names its exit, and the recover the host starts as its operator
    /// lands it in the bay. The surface clock is taken to the recover's own instant, never
    /// stepped to it.
    async fn goes_home(&mut self, unit: ObjId, secs: u64) -> bool {
        let out = self
            .wait(secs, |rig, _| {
                !in_field(rig, unit) && site_record(rig).is_some_and(|record| record.exited(unit))
            })
            .await;
        if !out {
            return false;
        }
        self.wait(120, |rig, _| {
            let due = record_of(&mut rig.tw, unit)
                .and_then(|record| record.act)
                .map(|act| act.due_ic);
            if let Some(due) = due {
                to_ic(rig, due);
            }
            matches!(
                parent(&rig.tw, unit),
                mush_sim::SpatialParent::Interior { .. }
            )
        })
        .await
    }

    /// Where `unit` is and what its record holds, for a failed wait.
    fn state_of(&mut self, unit: ObjId) -> String {
        let record = record_of(&mut self.rig.tw, unit);
        format!(
            "#{}: in the field {}, exit on the record {}, at {:?}, parent {:?}, act {:?}, last {:?}, queued {}, plan {:?}",
            unit.0,
            in_field(&self.rig, unit),
            site_record(&mut self.rig).is_some_and(|site| site.exited(unit)),
            self.cell(unit),
            parent(&self.rig.tw, unit),
            record
                .as_ref()
                .and_then(|record| record.act.as_ref().map(|act| (act.kind, act.due_ic))),
            record
                .as_ref()
                .and_then(|record| record.last.as_ref().map(|last| (last.kind, last.result))),
            record
                .as_ref()
                .is_some_and(|record| record.queued.is_some()),
            record
                .as_ref()
                .and_then(|record| record.plan.as_ref().map(|plan| plan.state)),
        )
    }

    /// The cell `unit` stands on, on the site's ground.
    fn cell(&self, unit: ObjId) -> Option<(i64, i64)> {
        mush_sim::pose_of(self.rig.tw.world(), unit)
            .map(|(_, pose)| tacsim::surface::excursion::site_cell(pose.x_cm, pose.y_cm))
    }

    /// Whether the site has demoted.
    fn demoted(&mut self) -> bool {
        task(&self.rig).is_none()
            && site_record(&mut self.rig).is_some_and(|record| record.promoted.is_none())
    }

    /// What the black box holds, for a failed wait.
    fn exchange(&self) -> String {
        format!(
            "the armed rig fired {}; the intruder fired {}, arrived on the armed rig {}, on the prospector {}, on the second miner's {}",
            fired(&self.lines, self.escort),
            fired(&self.lines, self.hostile),
            arrived(&self.lines, self.hostile, self.escort),
            arrived(&self.lines, self.hostile, self.digger),
            arrived(&self.lines, self.hostile, self.worker),
        )
    }
}

/// Whether `text` is `template` with its `{names}` filled: the literal pieces of the
/// template, in order.
fn fills(text: &str, template: &str) -> bool {
    let mut pieces = Vec::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        pieces.push(&rest[..open]);
        rest = rest[open..]
            .find('}')
            .map_or("", |close| &rest[open + close + 1..]);
    }
    pieces.push(rest);
    let mut at = 0;
    pieces
        .iter()
        .filter(|piece| !piece.is_empty())
        .all(|piece| {
            text[at..]
                .find(piece)
                .map(|found| at += found + piece.len())
                .is_some()
        })
}

/// **What the field's last snapshot holds of `unit`**: whether the kernel flies it for an
/// absent player, and the posture the kernel was told. `None` with no snapshot yet, or no
/// such unit in it.
fn saved(rig: &Rig, unit: ObjId) -> Option<(bool, Posture)> {
    let document = tacsim::field::snapshot_on(rig.tw.world(), &cid(AHUNA))?;
    // The state rides the document as a value or as its own text; read it either way.
    let state = match document.get("state")? {
        serde_json::Value::String(text) => serde_json::from_str(text).ok()?,
        other => other.clone(),
    };
    let row =
        state.get("units")?.as_array()?.iter().find(|row| {
            row.get("id").and_then(serde_json::Value::as_u64) == Some(u64::from(unit.0))
        })?;
    let absent = row
        .get("absent")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let posture = match row.get("posture") {
        None => Posture::Return,
        Some(word) => Posture::parse(word.as_str()?)?,
    };
    Some((absent, posture))
}

/// **Story 1** (this module's head): an unattended rig on a raided site does not fire first,
/// goes home, and its operator reads the plan's end at login.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn ap7_an_unattended_rig_on_a_raided_site_goes_home_and_its_operator_reads_the_plans_end() {
    let mut s = site(Posture::Return, quiet).await;
    let (owner, escort, digger, worker, hostile) =
        (s.owner, s.escort, s.digger, s.worker, s.hostile);
    // From the turn the field took them in, each is its mind's as an absent player's unit.
    for unit in [escort, digger, worker, s.carrier] {
        assert_eq!(
            away_of(s.rig.tw.world(), unit).map(|away| (away.why, away.handed)),
            Some((Why::Unattended, true)),
            "#{} nobody sits in is its mind's from admission",
            unit.0
        );
        assert!(minded(s.rig.tw.world(), unit));
    }
    // The raid: the intruder's fire lands on the second miner's rig, beside the armed one.
    let struck = s
        .raid(WORKER_AT, 90, |_, lines| {
            arrived(lines, hostile, worker) >= 2
        })
        .await;
    assert!(
        struck,
        "the intruder's fire arrives on the second miner's rig: {}",
        s.exchange()
    );
    // Both of the operator's rigs go home and are put aboard.
    for unit in [escort, digger] {
        let home = s.goes_home(unit, 240).await;
        assert!(
            home,
            "a rig on RETURN is let out at its carrier's ramp and put aboard: {}; {}",
            s.state_of(unit),
            s.exchange()
        );
        assert!(s.aboard(unit), "#{} is in the carrier's bay", unit.0);
    }
    // The second miner's rig, on HOLD, did not: it is still in the field, never let out.
    assert!(
        in_field(&s.rig, worker)
            && site_record(&mut s.rig).is_some_and(|record| !record.exited(worker)),
        "a rig on HOLD is not taken home: {}",
        s.state_of(worker)
    );
    let _ = s.wait(3, |_, _| false).await;
    // The armed rig fired on nobody: not on the intruder its co-op declared Hostile, in range
    // of it for the whole of its drive, and not for the stranger's rig beside it.
    assert_eq!(
        fired(&s.lines, escort),
        0,
        "an unattended rig does not fire first: {}",
        s.exchange()
    );
    assert_eq!(arrived(&s.lines, hostile, escort), 0, "{}", s.exchange());
    // The plan ended when its rig came aboard, and says why.
    let plan = record_of(&mut s.rig.tw, digger)
        .and_then(|record| record.plan)
        .expect("the plan is kept on the record");
    assert!(
        matches!(
            plan.state,
            FillState::Ended {
                why: PlanEnd::Field,
                ..
            }
        ),
        "the plan ends once its rig is aboard: {plan:?}"
    );
    assert_eq!(plan.tally.cycles, 0, "and it mined nothing meanwhile");
    // He logs in and reads, queued at trip level: that a field opened around each rig, and
    // that the plan ended, with why.
    let heard = connect(&mut s.rig, owner);
    for unit in [escort, digger] {
        let opened = UNATTENDED.replace("{unit}", &unit.0.to_string());
        assert!(heard.contains(&opened), "{opened}: {heard}");
    }
    assert!(
        heard
            .lines()
            .any(|text| fills(text.trim(), PLAN_ENDED) && text.contains(END_FIELD)),
        "the plan's end, with why: {heard}"
    );
    assert!(
        !heard.contains(&format!("A field opened around #{} with", s.carrier.0)),
        "a carrier stays where it is, and nothing is said of its mind: {heard}"
    );
}

/// The field demotes when nobody has fired for twenty seconds.
fn brief(cfg: &mut tacsim::TacsimConfig) {
    tune(cfg);
    cfg.field.disengage_s = 20;
    cfg.field.idle_demote_s = 20;
}

/// **Story 2** (this module's head): the same rig on `HOLD` is still there at demotion, and
/// its plan runs on.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn ap7_the_same_rig_on_hold_is_still_there_at_demotion_and_its_plan_runs_on() {
    let mut s = site(Posture::Hold, brief).await;
    let (owner, escort, digger, stand) = (s.owner, s.escort, s.digger, s.stand);
    assert_eq!(
        away_of(s.rig.tw.world(), digger).map(|away| (away.why, away.handed)),
        Some((Why::Unattended, true)),
    );
    // The field runs and closes with nobody firing. While it runs the plan starts nothing.
    let mut started = false;
    assert!(
        s.wait(120, |rig, _| {
            let record = record_of(&mut rig.tw, digger);
            if task(rig).is_some() {
                started |=
                    record.is_some_and(|record| record.act.is_some() || record.run.is_some());
            }
            task(rig).is_none() && site_record(rig).is_some_and(|record| record.promoted.is_none())
        })
        .await,
        "the site demotes"
    );
    assert!(!started, "the plan starts nothing while the field runs");
    assert!(s.demoted());
    // The rig is where it stood, on the ground, never taken out at the ramp.
    assert_eq!(
        s.cell(digger),
        Some(stand),
        "a rig on HOLD is still on its cell at demotion"
    );
    assert!(!in_field(&s.rig, digger) && !s.aboard(digger));
    assert_eq!(fired(&s.lines, escort), 0, "{}", s.exchange());
    let plan = record_of(&mut s.rig.tw, digger)
        .and_then(|record| record.plan)
        .expect("the plan");
    assert_eq!(
        (plan.state, plan.cursor),
        (FillState::Live, 0),
        "the plan is as it was filed"
    );
    // The plan runs on: the rig's next act is the plan's own, started by the surface tick as
    // its author, who is still away. The clock is taken a minute at a time past any wait the
    // plan reads, never stepped second by second.
    let mut by = None;
    for _ in 0..40 {
        let _ = second(&mut s.rig);
        let record = record_of(&mut s.rig.tw, digger).expect("the rig's record");
        by = record
            .run
            .as_ref()
            .map(|run| (ActKind::Mine, run.by))
            .or_else(|| record.act.as_ref().map(|act| (act.kind, act.by)));
        if by.is_some() {
            break;
        }
        let now = ic_now(&s.rig.tw);
        to_ic(&mut s.rig, now + 60);
    }
    assert!(
        matches!(by, Some((_, Some(ActBy::Plan { no: 1, .. })))),
        "after the demotion the rig's next act is its plan's: {by:?}"
    );
    assert!(!s.rig.tw.world().is_connected(owner));
    // He reads at login that the field opened around it and that it stayed.
    let heard = connect(&mut s.rig, owner);
    let stayed = UNATTENDED_ON
        .replace("{unit}", &digger.0.to_string())
        .replace("{posture}", Posture::Hold.label());
    assert!(heard.contains(&stayed), "{stayed}: {heard}");
    assert!(
        !heard.lines().any(|text| fills(text.trim(), PLAN_ENDED)),
        "no plan ended: {heard}"
    );
}

/// **Story 3** (this module's head): a carrier with nobody aboard is still on its pad with
/// its returned rigs in its bay.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ap7_a_carrier_with_nobody_aboard_stays_on_its_pad_and_takes_its_rigs_aboard() {
    // A prospector on GUARD has nothing to fire: it stays, as on HOLD (Q-45-10).
    let mut s = site(Posture::Guard, quiet).await;
    let (carrier, escort, digger, stand) = (s.carrier, s.escort, s.digger, s.stand);
    let pad =
        mush_sim::pose_of(s.rig.tw.world(), carrier).map(|(on, pose)| (on, pose.x_cm, pose.y_cm));
    assert_eq!(
        away_of(s.rig.tw.world(), carrier).map(|away| (away.why, away.handed)),
        Some((Why::Unattended, true)),
        "the carrier is its mind's from admission"
    );
    // Its armed rig comes home on RETURN and is taken into the bay.
    assert!(
        s.goes_home(escort, 240).await,
        "the armed rig is let out at the ramp and put aboard: {}",
        s.exchange()
    );
    assert!(s.aboard(escort));
    // The kernel flies the carrier for an absent player, standing: its record says RETURN,
    // and a carrier on its pad is home, so its arena was told HOLD (Q-45-11).
    assert_eq!(
        tacsim::posture::record::of(s.rig.tw.world(), carrier),
        Posture::Return
    );
    assert!(
        s.wait(30, |rig, _| saved(rig, carrier)
            == Some((true, Posture::Hold)))
            .await,
        "the field's snapshot holds the carrier as an absent player's unit on HOLD: {:?}",
        saved(&s.rig, carrier)
    );
    // A minute and a half on, long enough for a prospector on RETURN to have gone home from
    // this cell (story 1's does): the prospector on GUARD, which carries nothing to fire, was
    // never let out.
    let left = s
        .wait(90, |rig, _| {
            !in_field(rig, digger) || site_record(rig).is_some_and(|record| record.exited(digger))
        })
        .await;
    assert!(
        !left,
        "an unarmed rig on GUARD is not taken home: {}",
        s.state_of(digger)
    );
    assert!(
        s.stands(digger, stand) && !s.aboard(digger),
        "it stays where it stood"
    );
    assert_eq!(
        saved(&s.rig, digger),
        Some((true, Posture::Guard)),
        "its word is kept: the kernel flies it as HOLD and the record still says GUARD"
    );
    // And the carrier has gone nowhere: nothing asked it to.
    assert!(
        in_field(&s.rig, carrier)
            && site_record(&mut s.rig).is_some_and(|record| !record.exited(carrier)),
        "the carrier is still in the field, on its pad"
    );
    assert_eq!(
        mush_sim::pose_of(s.rig.tw.world(), carrier).map(|(on, pose)| (on, pose.x_cm, pose.y_cm)),
        pad,
        "where it was settled"
    );
    assert!(
        !s.rig.tw.world().is_connected(s.owner),
        "its commander is still away"
    );
    assert_eq!(fired(&s.lines, escort), 0, "{}", s.exchange());
}

/// **Story 4** (this module's head; doc 45 unit AP.11's finding): a rig nobody sits in is put
/// aboard when its operator sits in the cab of his other rig. The recover is a hand act its
/// operator gives from aboard the carrier or from that rig's own cab; the host's recover of
/// a unit it took out for an absent player is run as that operator, wherever he is.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ap7_a_rig_whose_operator_sits_in_his_other_rig_is_still_put_aboard() {
    let mut rig = stocked_tuned(quiet);
    // The lessee in his armed rig's cab; his prospector three cells off, nobody in it, five
    // cells from the ramp.
    let (lessee, hull, _, _, others) = stand_up_out(&mut rig, 1, Some((133, 125)));
    let kin = others[0];
    let _ = rig.tw.connect(lessee);
    for _ in 0..3 {
        let _ = second(&mut rig);
    }
    // `stand_up_out` left the prospector on HOLD. From the armed rig's cab he gives it RETURN
    // again, over the relay (doc 45 unit AP.4).
    let set = run(&mut rig.tw, lessee, "+tv/2 posture return");
    assert_eq!(
        tacsim::posture::record::of(rig.tw.world(), kin),
        Posture::Return,
        "{set}"
    );
    let seated = rig
        .tw
        .world()
        .module_state::<mush_sim::SpatialState>()
        .and_then(|state| state.assignments.get(&hull))
        .and_then(|host| host.seats.values().next())
        .map(|seat| seat.actor);
    assert_eq!(
        seated,
        Some(lessee),
        "the operator sits in his armed rig, not aboard the carrier"
    );
    let mut rx = egress(&mut rig);
    let mut lines = Vec::new();
    // His own mark opens the field, and he stays at his controls.
    let (out, queued) = command(&mut rig, lessee, "+field/mark E=hostile");
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    land(&mut rig, &queued[0]).expect("the promotion lands");
    assert!(
        until(&mut rig, &mut rx, &mut lines, 15, |rig, _| task(rig)
            .is_some())
        .await,
        "the field's task opens"
    );
    assert_eq!(
        away_of(rig.tw.world(), kin).map(|away| (away.why, away.handed)),
        Some((Why::Unattended, true)),
        "the prospector is its mind's from admission"
    );
    assert_eq!(
        away_of(rig.tw.world(), hull),
        None,
        "its operator holds his own rig's seat"
    );
    // Its mind takes it to the ramp, and the host puts it aboard.
    let out = until(&mut rig, &mut rx, &mut lines, 240, |rig, _| {
        !in_field(rig, kin) && site_record(rig).is_some_and(|record| record.exited(kin))
    })
    .await;
    assert!(out, "the prospector is let out at its carrier's ramp");
    let home = until(&mut rig, &mut rx, &mut lines, 120, |rig, _| {
        let due = record_of(&mut rig.tw, kin)
            .and_then(|record| record.act)
            .map(|act| act.due_ic);
        if let Some(due) = due {
            to_ic(rig, due);
        }
        matches!(
            parent(&rig.tw, kin),
            mush_sim::SpatialParent::Interior { .. }
        )
    })
    .await;
    let record = record_of(&mut rig.tw, kin);
    assert!(
        home,
        "the host puts it aboard as its operator, who sits in his other rig: last {:?}",
        record.and_then(|record| record.last.map(|last| (last.kind, last.result)))
    );
    assert!(
        matches!(
            tacsim::surface::act::where_is(rig.tw.world(), kin),
            tacsim::surface::act::Where::Bay { .. }
        ),
        "it is in its carrier's bay"
    );
}

//! **Doc 45 unit AP.11 — the closing story** (Q-45-14, Q-45-18, Q-45-22; with it the
//! programme's own rules, doc 45 §7), over the composed world (`driving`'s keyed rig on Ahuna
//! Flank) with the arena's egress pumped by hand.
//!
//! One story, [`ap11_an_absent_players_outfit_through_a_raid_and_their_return`], and one
//! field of four parties. Every command is typed by a player who is not staff.
//!
//! **The cast.**
//!
//! - **The owner**: a carrier on the pad, a leased armed rig he sits in, and a prospector out
//!   on the ground with nobody in its cab. He sets the armed rig to `GUARD` from its cab,
//!   leaves the prospector on `RETURN` (the word every unit holds until its player says
//!   otherwise), and quits in the armed rig's seat. A third hull he holds title to lies
//!   disabled on open ground: a casualty of a fight already had.
//! - **The miner**, connected throughout: a prospector on `HOLD` beside the owner's armed rig.
//!   While the owner is away her rig tows his casualty into her own carrier and she files a
//!   salvage claim on it.
//! - **The raider**: an armed hull ten cells east, whose player types every lock and every
//!   shot. The owner's co-op declared the raider's Hostile, by his own command, before he
//!   left, so every unit of his holds the raider Hostile from the field's first pulse. The
//!   raider's mark on the flight to the west opens the field with no round fired (Q-45-14:
//!   an attacker may start a fight at any time, whoever is connected).
//!
//! **The walk, and what each step pins.**
//!
//! 1. *Admission.* The owner is not admitted as the armed rig's pilot: the rig is its mind's
//!    from the turn the field takes it in, and so are his prospector and his carrier.
//! 2. *A stranger's quarrel.* The raider's fire lands on the miner's rig beside the guard.
//!    The guard fires nothing, with the raider declared Hostile and in range the whole time.
//! 3. *`RETURN` goes home.* The prospector drives to the ramp and is put aboard the carrier,
//!    by the host, as its operator, who sits in his other rig (the defect this story found:
//!    `crate::field_ap7`'s story 4). The miner's prospector, the same design on `HOLD` on the
//!    same ground, stays.
//! 4. *Its owner's unit.* The raider's fire lands on the owner's carrier. The guard returns
//!    it, on the raider and on nobody else, and stays where it stood.
//! 5. *The return.* The owner connects while the field runs. He reads what the game queued
//!    for him: that the field opened, and what his mind was left to do with each rig. He is
//!    seated again, and the first command he types is what the rig does: it leaves the cell
//!    its mind was standing on, and when the raider fires on the carrier again, the fire its
//!    mind answered, it fires nothing.
//! 6. *The close.* The field settles with the owner connected, and he is told what became of
//!    each unit: the guard holding, the prospector and the carrier home.
//! 7. *The field's own record of fire*, read in order over the whole black box: the first
//!    round of the field is the raider's, no unit of the owner's fired before the raider's
//!    fire had arrived on a unit of his, and every round of his went at the raider.
//! 8. *The same words.* The three postures read back off the three units' own records after
//!    the demotion and after a restart.
//! 9. *The casualty* (Q-45-22). The claim's contest window is the configured 48 IC days,
//!    counted from the second it was filed, while the title holder was away. His login moves
//!    nothing, and it is still what it was after the restart. He was written to at the filing.
//!
//! **The ship arm, and what it owes doc 45 unit AP.9.** The owner's ship in this story is his
//! carrier. Only this much of the row's "a ship on `HOLD`" is true today and pinned here: her
//! posture is the same one record a rig's is (unit AP.3), it reads `HOLD` through the field's
//! promotion and demotion and after a restart, and nothing the field does moves it. **Not
//! pinned, because it is not built:** no verb sets a ship's posture outside a field (`+tv`
//! addresses rigs, and `MIND POSTURE` is a seat's word inside a field), so the story writes
//! her record through the record's one writer, as AP.3's story did; and a ship flies no
//! posture in a bubble. AP.9 adds both to **this** story, at the two places marked `AP.9`:
//! the owner's own words in place of the staged write, and a ship in a bubble that complies
//! under `HOLD` while the rigs here fly theirs.
//!
//! **What is a fixture** (each named in the cut note): the raider's hull is made and seated
//! by a staff hand (CF.2's `armed` and `seat`); the two co-ops are made and their officers
//! named by staff (the stance is the owner's own command); rigs are stood on their cells by staff
//! (`to_cell`); the casualty is a spare rig titled to the owner by `@asset/transfer`, its
//! power destroyed and left on the site's index by staff (GC.6's precedent: a leased rig's
//! title cannot move, and no verb disables a hull); the world's rate is set to the live
//! game's 24:1 by staff's `@tacsim/time`; the ship's posture is the staged write above; and
//! the mail is the seam the server fills with `mush-mail`, filled here with a recorder.

use std::sync::Arc;

use mush_core::{GOD, ObjId};
use mush_economy::{ContractId, ContractState, EconomyStore};
use mush_org::PrincipalId;
use tacsim::arena::ledger::{EventKind, LedgerEvent};
use tacsim::arena::perception::FrameContact;
use tacsim::arena_task::ArenaOut;
use tacsim::field::absent::{ADMITTED_ON, BACK_IN_SEAT, UNATTENDED, Why, away_of, minded};
use tacsim::field::notice::{AFTER_FIELD, Fate};
use tacsim::field::{FIELD_OPENED, FIELD_SETTLED};
use tacsim::port_tick::{NOTICE_HEADING, pending_notices};
use tacsim::posture::Posture;
use tacsim::posture::order::{POSTURE_READ, POSTURE_SET, line};
use tacsim::surface::salvage::{ClaimSeam, SalvageCustody};
use tokio::sync::mpsc::Receiver;

use crate::driving_rig::{
    RAMP, RAMP_CELL, Rig, act_go, cid, command, count, finish, land, launched, on_ground_at,
    on_pad_as, on_pad_with, parent, reboot, record_of, run, said, set_rate, stocked_tuned, to_cell,
    to_ic, warm, world_rate,
};
use crate::escort_lease::{board_escort, contact_near, fired, in_field, lease_escort, top_up};
use crate::field_ap1::{Heard, Post, until};
use crate::field_ap7::officer;
use crate::field_gc4::{powerless, seconds_until, towing};
use crate::field_host_rig::{
    AHUNA, armed, egress, frame, seat, second, site_record, task, tune, weapon_of,
};
use crate::field_mw7::{arrived, bound, connect, fired_at, quit};
use crate::site_record_rig::{staff, staff_member};

/// Where the owner's armed rig stands on `GUARD`, his prospector on `RETURN`, the miner's
/// prospector on `HOLD`, the raider, and the owner's casualty.
const GUARD_AT: (u16, u16) = (136, 122);
const HOMER_AT: (i64, i64) = (133, 125);
const WORKER_AT: (u16, u16) = (140, 120);
const RAIDER_AT: (i64, i64) = (146, 122);
const CASUALTY_AT: (i64, i64) = (120, 112);
/// Where the owner sends the armed rig with his first command.
const GOAL: (u16, u16) = (132, 119);
/// Seconds of arena time with no shot fired before the field demotes: longer than any quiet
/// stretch of the story, and what the close then costs.
const IDLE_S: u32 = 60;
/// The live game's rate, IC seconds a real second (the owner's 2026-11-11 rule).
const LIVE_RATE: u32 = 24;
/// The salvage contest window Q-45-22 leaves as it was, IC days.
const WINDOW_IC_DAYS: i64 = 48;

/// The field's tempo: the lab's pulse, a field that holds until nobody has fired for
/// [`IDLE_S`], a short rally (a mind that would fire first has done so well inside each quiet
/// stretch), a black box that reaches the story while the field runs, and short tow stages.
/// The claim's window is left as shipped.
fn tempo(cfg: &mut tacsim::TacsimConfig) {
    tune(cfg);
    cfg.field.disengage_s = 3_600;
    cfg.field.idle_demote_s = IDLE_S;
    cfg.arena.reflex.rally_ms = 4_000;
    cfg.arena.ledger.summary_ms = 100;
    cfg.recovery.inspect_s = 30;
    cfg.recovery.attach_s = 30;
    cfg.recovery.load_s = 60;
}

/// The posture `unit`'s own record holds (doc 45 unit AP.3: one record, on the unit).
fn held(rig: &Rig, unit: ObjId) -> Posture {
    tacsim::posture::record::of(rig.tw.world(), unit)
}

/// `obj`'s name, as a notice prints it.
fn name(rig: &Rig, obj: ObjId) -> String {
    rig.tw
        .world()
        .get(obj)
        .map(|object| object.name.clone())
        .expect("a named object")
}

/// The cell `unit` stands on, on the site's ground.
fn cell(rig: &Rig, unit: ObjId) -> Option<(i64, i64)> {
    mush_sim::pose_of(rig.tw.world(), unit)
        .map(|(_, pose)| tacsim::surface::excursion::site_cell(pose.x_cm, pose.y_cm))
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

/// `line` as `who`, the one submission it queued landed; the reply.
fn go(rig: &mut Rig, who: ObjId, line: &str) -> String {
    let (out, queued) = command(rig, who, line);
    assert_eq!(queued.len(), 1, "{line}: one submission: {out}");
    land(rig, &queued[0]).unwrap_or_else(|error| panic!("{line}: {error}"));
    out
}

/// The custody row that names `hull` on Ahuna Flank's record.
fn custody(rig: &mut Rig, hull: ObjId) -> Option<SalvageCustody> {
    site_record(rig)?
        .custody
        .into_iter()
        .find(|row| row.hull == hull)
}

/// The claim's economy contract.
fn contract(rig: &Rig, id: u64) -> mush_economy::EconomicContract {
    rig.tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|economy| economy.snapshot().contract(ContractId(id)).cloned())
        .expect("the claim's contract")
}

/// Who holds title to `hull`.
fn title(rig: &Rig, hull: ObjId) -> Option<PrincipalId> {
    rig.tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|economy| {
            economy
                .snapshot()
                .asset(hull)
                .map(|a| a.title_owner.clone())
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

/// A contact plotted within two cells of `cell`.
fn at(cell: (u16, u16)) -> impl Fn(&FrameContact) -> bool {
    move |row| {
        row.cell
            .is_some_and(|(x, y)| x.abs_diff(cell.0) <= 2 && y.abs_diff(cell.1) <= 2)
    }
}

/// The first line of `lines` that `pick` takes.
fn first(lines: &[LedgerEvent], pick: impl Fn(&EventKind) -> bool) -> Option<usize> {
    lines.iter().position(|line| pick(&line.kind))
}

/// **The owner's outfit, the miner and the raider on Ahuna Flank**, in a running field the
/// owner was away for (this module's head).
struct Story {
    rig: Rig,
    /// The owner, his carrier (his ship), his armed rig on `GUARD` and his prospector on
    /// `RETURN`.
    owner: ObjId,
    carrier: ObjId,
    guard: ObjId,
    homer: ObjId,
    /// The miner and her prospector on `HOLD`: the stranger beside the guard.
    miner: ObjId,
    worker: ObjId,
    /// The raider's player and hull.
    intruder: ObjId,
    hostile: ObjId,
    /// The owner's casualty, the claim's contract, and the world second it was filed.
    casualty: ObjId,
    claim: u64,
    filed_at: i64,
    /// Every letter the mail seam was handed.
    post: Post,
    rx: Receiver<ArenaOut>,
    lines: Vec<LedgerEvent>,
    /// Everything the world said, to anyone, since the field's task opened.
    heard: Heard,
}

/// **Stand the story up to the field's first pulse** (this module's head, the cast).
#[allow(clippy::too_many_lines)] // one staging, in the order a player would do it
async fn stand_up() -> Story {
    let mut rig = stocked_tuned(tempo);
    let post: Post = Arc::default();
    {
        let post = Arc::clone(&post);
        assert!(
            tacsim::surface::claim::install_claim_mail(
                rig.tw.world_mut(),
                Arc::new(
                    move |_ctx: &mut mush_core::Ctx<'_>,
                          _from: ObjId,
                          to: &[ObjId],
                          subject: &str,
                          body: &str| {
                        post.lock().map_err(|error| error.to_string())?.push((
                            to.to_vec(),
                            subject.to_owned(),
                            body.to_owned(),
                        ));
                        Ok(())
                    },
                ),
            ),
            "the mail seam is composed"
        );
    }
    // The miner: a carrier on the pad and one prospector, launched.
    let (miner, _, miner_tvs) = on_pad_as(&mut rig, "Miner", 2, "Tally Moth", 1);
    let worker = miner_tvs[0];
    launched(&mut rig, miner, worker);
    // The owner's outfit: the armed lease in slot 1 and a prospector in slot 2.
    let (owner, row, tvs) = on_pad_with(&mut rig, "Pilot", 1, "Salt Moth", |rig, pilot| {
        lease_escort(rig, pilot);
        // The escort's deposit leaves too little for a second lease: staff mint it.
        top_up(rig, pilot, 1_000_000);
        crate::driving_rig::lease_one(rig, pilot);
    });
    let carrier = ObjId(row.hull);
    let (guard, homer) = (tvs[0], tvs[1]);
    // His prospector goes down the ramp by his own command from aboard, and is stood off it.
    // He sets nothing on it: it holds RETURN, and he reads that from aboard its carrier.
    let launch = act_go(&mut rig, owner, "+tv/2 launch go");
    assert!(!launch.is_empty(), "{launch}");
    let _ = finish(&mut rig, homer, owner);
    assert!(on_ground_at(&rig.tw, homer, RAMP), "rig 2 is down the ramp");
    to_cell(&mut rig, homer, HOMER_AT);
    assert_eq!(
        run(&mut rig.tw, owner, "+tv/2 posture"),
        line(POSTURE_READ, "Rig 2", Posture::Return),
        "a rig nobody has given a word holds RETURN"
    );
    // He boards the armed rig in the bay, takes it down the ramp and drives it out.
    board_escort(&mut rig, owner, guard);
    launched(&mut rig, owner, guard);
    assert!(on_ground_at(&rig.tw, guard, RAMP), "rig 1 is down the ramp");
    let drive = act_go(
        &mut rig,
        owner,
        &format!("+tv/1 drive {},{} go", GUARD_AT.0, GUARD_AT.1),
    );
    assert!(!drive.is_empty(), "{drive}");
    let _ = finish(&mut rig, guard, owner);
    // The raider: a third player's armed hull standing to the east.
    let (intruder, _, _) = on_pad_as(&mut rig, "Intruder", 3, "Lone Moth", 0);
    let hostile = armed(&mut rig, "Grey Hound", RAIDER_AT, 270_000);
    seat(&mut rig, hostile, intruder);
    // The owner's co-op declares the raider's Hostile, by his own command, before any of it
    // (doc 40 cut FR.1): each unit of his holds the raider Hostile from the field's first
    // pulse. A mind fires only on what its party holds Hostile, so without this a guard's
    // silence beside a stranger's quarrel would show nothing about its posture.
    for faction in ["Concord", "Basin"] {
        let out = run(&mut rig.tw, GOD, &format!("@faction/create {faction}"));
        assert!(out.contains(faction), "{out}");
    }
    officer(&mut rig, "Concord", owner);
    officer(&mut rig, "Basin", intruder);
    let declared = run(&mut rig.tw, owner, "+faction/stance Concord=Basin/hostile");
    assert!(declared.contains("Hostile"), "{declared}");
    warm(&mut rig);
    // The owner's casualty: a spare rig of Occator's pool, titled to him by staff (a leased
    // rig's title carries the lease's claim and cannot move), disabled and left on open ground.
    let schema = mush_tactics::module::unit_of(rig.tw.world(), worker)
        .expect("the miner's unit")
        .schema()
        .clone();
    let casualty = rig
        .tw
        .world()
        .module_state::<mush_tactics::UnitStore>()
        .expect("the units")
        .units
        .iter()
        .filter(|(id, unit)| ![worker, homer].contains(*id) && unit.schema() == &schema)
        .map(|(id, _)| *id)
        .find(|id| {
            crate::driving_rig::store(&rig.tw)
                .tv_lease_on(*id)
                .is_none()
        })
        .expect("a spare rig in Occator's pool");
    let moved = run(
        &mut rig.tw,
        GOD,
        &format!("@asset/transfer #{}=#{}", casualty.0, owner.0),
    );
    assert_eq!(
        title(&rig, casualty),
        Some(PrincipalId::Object(owner)),
        "{moved}"
    );
    to_cell(&mut rig, casualty, CASUALTY_AT);
    powerless(&mut rig, casualty);
    let hand = staff_member(&mut rig);
    staff(
        &mut rig,
        hand,
        &format!(
            "@tacsim/site {AHUNA} leave #{} casualty {},{}",
            casualty.0, CASUALTY_AT.0, CASUALTY_AT.1
        ),
    );
    // The live game's rate, so a window measured in IC days is what a player would wait.
    set_rate(&mut rig.tw, LIVE_RATE);
    let _ = rig.tw.connect(owner);
    for _ in 0..3 {
        let _ = second(&mut rig);
    }

    // The owner's words. The armed rig, from its own cab on open ground (doc 45 unit AP.4).
    assert_eq!(
        run(&mut rig.tw, owner, "+tv/1 posture guard"),
        line(POSTURE_SET, "Rig 1", Posture::Guard),
        "the set says what was set, and no more"
    );
    // AP.9: the ship. No verb sets a ship's posture outside a field today, so her record is
    // written through its one writer (this module's head). AP.9 replaces this with the
    // owner's own command.
    let staged = crate::support::with_ctx(&mut rig.tw, |ctx| {
        tacsim::posture::record::set(ctx, carrier, Posture::Hold)
    });
    assert_eq!(staged, Ok(true), "the ship's record moves to HOLD");
    assert_eq!(
        (held(&rig, guard), held(&rig, homer), held(&rig, carrier)),
        (Posture::Guard, Posture::Return, Posture::Hold),
        "three units of one player, three words, each on the unit's own record"
    );
    // He logs off in the armed rig's seat, on open ground. No field runs, so nothing is
    // handed over, and the seat still names him.
    quit(&mut rig, owner);
    assert!(!rig.tw.world().is_connected(owner));
    assert_eq!(
        sits_in(&rig, guard),
        Some(owner),
        "the seat outlives the quit"
    );
    assert_eq!(
        away_of(rig.tw.world(), guard),
        None,
        "no field holds the rig"
    );

    // While he is away the miner's rig tows his casualty into her carrier, and she files.
    to_cell(&mut rig, worker, (CASUALTY_AT.0 + 1, CASUALTY_AT.1));
    let _ = second(&mut rig);
    let out = go(&mut rig, miner, &format!("+tv/1 tow #{} go", casualty.0));
    assert!(
        seconds_until(&mut rig, 120, |rig| towing(rig, worker) == Some(casualty)),
        "the tow attaches: {out}"
    );
    to_cell(&mut rig, worker, RAMP_CELL);
    let _ = second(&mut rig);
    let out = go(&mut rig, miner, "+tv/1 load go");
    assert!(
        seconds_until(&mut rig, 120, |rig| custody(rig, casualty).is_some()
            && site_record(rig)
                .is_some_and(|record| record.tows.act_of(worker).is_none())),
        "the load lands in custody: {out}"
    );
    assert!(
        !rig.tw.world().is_connected(owner),
        "the owner is still away"
    );
    let filed_at = rig.tw.world().now();
    let out = go(
        &mut rig,
        miner,
        &format!("+salvage/claim #{} go", casualty.0),
    );
    assert!(out.contains("You file a salvage claim"), "{out}");
    let row = custody(&mut rig, casualty).expect("the casualty is in the miner's custody");
    let ClaimSeam::Filed { contract: claim } = row.claim else {
        panic!("the claim is filed: {row:?}");
    };
    assert_eq!(row.told, vec![owner], "the title holder is who is told");
    // And he is written to now, at the filing, while he is away: the one letter of the story.
    let posted: Vec<Vec<ObjId>> = post
        .lock()
        .expect("the post")
        .iter()
        .map(|(to, _, _)| to.clone())
        .collect();
    assert_eq!(
        posted,
        vec![vec![owner]],
        "one letter, the owner's, posted when the claim was filed"
    );
    // Her prospector goes back to work beside the owner's armed rig, and she leaves it on
    // HOLD from aboard her carrier: it is to stand through whatever comes.
    to_cell(
        &mut rig,
        worker,
        (i64::from(WORKER_AT.0), i64::from(WORKER_AT.1)),
    );
    assert_eq!(
        run(&mut rig.tw, miner, "+tv/1 posture hold"),
        line(POSTURE_SET, "Rig 1", Posture::Hold),
    );
    for _ in 0..3 {
        let _ = second(&mut rig);
    }

    // The raid lands: the raider marks the flight to its west Hostile, and the site promotes
    // with nobody having fired.
    let mut rx = egress(&mut rig);
    let mut lines = Vec::new();
    let mut heard: Heard = Vec::new();
    let (out, queued) = command(&mut rig, intruder, "+field/mark W=hostile");
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    land(&mut rig, &queued[0]).expect("the promotion lands");
    assert!(
        until(&mut rig, &mut rx, &mut lines, &mut heard, 15, |rig, _| {
            task(rig).is_some()
        })
        .await,
        "the field's task opens"
    );
    Story {
        rig,
        owner,
        carrier,
        guard,
        homer,
        miner,
        worker,
        intruder,
        hostile,
        casualty,
        claim,
        filed_at,
        post,
        rx,
        lines,
        heard,
    }
}

impl Story {
    /// Pump and tick until `done` holds or `secs` of wall time pass.
    async fn wait(
        &mut self,
        secs: u64,
        done: impl FnMut(&mut Rig, &[LedgerEvent]) -> bool,
    ) -> bool {
        until(
            &mut self.rig,
            &mut self.rx,
            &mut self.lines,
            &mut self.heard,
            secs,
            done,
        )
        .await
    }

    /// **The raider's player keeps a lock on the contact `pick` takes off their own cockpit
    /// and fires** until `done` holds or `secs` of wall time pass, then ceases: the lines a
    /// player types, once a second.
    async fn raid(
        &mut self,
        pick: impl Fn(&FrameContact) -> bool,
        secs: u64,
        mut done: impl FnMut(&Rig, &[LedgerEvent]) -> bool,
    ) -> bool {
        let gun = weapon_of(&self.rig, self.hostile);
        let mut held = false;
        for _ in 0..secs {
            let seen = frame(&self.rig, self.intruder)
                .and_then(|frame| frame.contacts.iter().find(|row| pick(row)).cloned());
            if let Some(contact) = seen
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

    /// Whether `unit` is in the owner's carrier's bay.
    fn aboard(&self, unit: ObjId) -> bool {
        tacsim::surface::act::where_is(self.rig.tw.world(), unit)
            == (tacsim::surface::act::Where::Bay { hull: self.carrier })
    }

    /// **`unit` is let out at its carrier's ramp and put aboard**, within `secs`: the kernel
    /// lets it go, the record names its exit, and the recover the host starts lands it in the
    /// bay. The surface clock is taken to the recover's own instant, never stepped to it.
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
            "#{}: in the field {}, exit on the record {}, at {:?}, act {:?}, last {:?}, queued {}",
            unit.0,
            in_field(&self.rig, unit),
            site_record(&mut self.rig).is_some_and(|site| site.exited(unit)),
            cell(&self.rig, unit),
            record
                .as_ref()
                .and_then(|record| record.act.as_ref().map(|act| (act.kind, act.due_ic))),
            record
                .as_ref()
                .and_then(|record| record.last.as_ref().map(|last| (last.kind, last.result))),
            record
                .as_ref()
                .is_some_and(|record| record.queued.is_some()),
        )
    }

    /// What the raider's cockpit plots, for a failed lock.
    fn plot(&self) -> String {
        frame(&self.rig, self.intruder).map_or_else(
            || "no frame".to_owned(),
            |frame| {
                frame
                    .contacts
                    .iter()
                    .map(|contact| format!("{} at {:?}", contact.designator, contact.cell))
                    .collect::<Vec<_>>()
                    .join("; ")
            },
        )
    }

    /// What the black box holds, for a failed wait.
    fn exchange(&self) -> String {
        format!(
            "the guard fired {} (at the raider {}), the prospector {}, the carrier {}; the \
             raider fired {}, arrived on the guard {}, on the prospector {}, on the carrier \
             {}, on the miner's rig {}",
            fired(&self.lines, self.guard),
            fired_at(&self.lines, self.guard, self.hostile),
            fired(&self.lines, self.homer),
            fired(&self.lines, self.carrier),
            fired(&self.lines, self.hostile),
            arrived(&self.lines, self.hostile, self.guard),
            arrived(&self.lines, self.hostile, self.homer),
            arrived(&self.lines, self.hostile, self.carrier),
            arrived(&self.lines, self.hostile, self.worker),
        )
    }
}

/// One fate's line of the close on Ahuna Flank, up to the units it names.
fn after(fate: Fate) -> String {
    let filled = AFTER_FIELD
        .replace("{n}", "1")
        .replace("{site}", "Ahuna Flank")
        .replace("{fate}", fate.word());
    filled
        .split("{units}")
        .next()
        .unwrap_or_default()
        .to_owned()
}

/// **The story** (this module's head).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // one field, in order
async fn ap11_an_absent_players_outfit_through_a_raid_and_their_return() {
    let mut s = stand_up().await;
    let (owner, carrier, guard, homer) = (s.owner, s.carrier, s.guard, s.homer);
    let (miner, worker, hostile) = (s.miner, s.worker, s.hostile);
    assert_eq!(
        world_rate(&s.rig.tw),
        LIVE_RATE,
        "the world runs at the live game's rate"
    );

    // (1) Admission. Four parties stand in the field, and the owner is not one of its pilots.
    let live = tacsim::field::live_on(s.rig.tw.world(), &cid(AHUNA)).expect("the live record");
    let parties: std::collections::BTreeSet<_> = [guard, homer, worker, hostile]
        .into_iter()
        .filter_map(|unit| live.party_of(unit))
        .collect();
    assert_eq!(
        parties.len(),
        4,
        "the owner's two rigs, the miner's and the raider's are four parties"
    );
    // His units hold the raider Hostile from the opening, by the stance he declared: what
    // keeps their fire from here on is the rule for an absent player's unit and nothing else.
    let theirs = live.party_of(hostile).expect("the raider's party");
    for unit in [guard, homer, carrier] {
        let ours = live.party_of(unit).expect("the unit's party");
        assert!(
            live.parties
                .iter()
                .any(|party| party.party == ours && party.stance_hostile.contains(&theirs)),
            "the declared stance is #{}'s party's opening relation: {:?}",
            unit.0,
            live.parties
        );
    }
    assert_eq!(
        away_of(s.rig.tw.world(), guard).map(|away| (away.why, away.handed)),
        Some((Why::AwayAtAdmission, true)),
        "a player logged off in the seat is not admitted as the rig's pilot"
    );
    assert!(
        minded(s.rig.tw.world(), guard) && !bound(&s.rig, owner),
        "the mind has the armed rig and nobody holds its seat"
    );
    for unit in [homer, carrier, worker] {
        assert_eq!(
            away_of(s.rig.tw.world(), unit).map(|away| (away.why, away.handed)),
            Some((Why::Unattended, true)),
            "#{} nobody sits in is its mind's from admission",
            unit.0
        );
    }
    let pad = mush_sim::pose_of(s.rig.tw.world(), carrier)
        .map(|(on, pose)| (on, pose.x_cm, pose.y_cm))
        .expect("the carrier stands on its pad");
    // The letter the field gave the carrier's party: what a raider reads of her on a cockpit.
    let ship_letter = live
        .party_of(carrier)
        .expect("the carrier's party")
        .letter()
        .to_string();
    let ship_again = ship_letter.clone();

    // (2) A stranger's quarrel: the raider's fire lands on the miner's rig, beside the guard.
    let struck = s
        .raid(at(WORKER_AT), 120, |_, lines| {
            arrived(lines, hostile, worker) >= 2
        })
        .await;
    assert!(
        struck,
        "the raider's fire arrives on the miner's rig: {}; plot {}",
        s.exchange(),
        s.plot()
    );

    // (3) RETURN goes home: the owner's prospector is let out at the ramp and put aboard.
    let went = s.goes_home(homer, 240).await;
    assert!(
        went,
        "a rig on RETURN is let out at its carrier's ramp and put aboard: {}; {}; the owner's \
         notices {:?}",
        s.state_of(homer),
        s.exchange(),
        pending_notices(s.rig.tw.world(), owner),
    );
    assert!(s.aboard(homer), "the prospector is in the carrier's bay");
    // The miner's prospector, the same design on HOLD on the same ground, was never let out.
    assert!(
        in_field(&s.rig, worker)
            && site_record(&mut s.rig).is_some_and(|record| !record.exited(worker)),
        "a rig on HOLD is not taken home"
    );
    // And through all of it the guard fired nothing and stood where it was set: the raider
    // its party holds Hostile in range the whole time, and a stranger's rig under fire beside
    // it.
    assert_eq!(
        fired(&s.lines, guard),
        0,
        "a guard holds its fire on a stranger's quarrel: {}",
        s.exchange()
    );
    assert!(
        in_field(&s.rig, guard)
            && contact_near(&s.rig, s.intruder, GUARD_AT)
                .is_some_and(|row| row.cell == Some(GUARD_AT)),
        "a guard stays where it stood: plot {}",
        s.plot()
    );
    assert_eq!(
        saved(&s.rig, guard),
        Some((true, Posture::Guard)),
        "the kernel flies it for an absent player, on GUARD"
    );

    // (4) Its owner's unit is fired on: the guard answers the unit that fired.
    let answered = s
        .raid(
            move |row| row.designator.to_string().starts_with(&ship_letter),
            150,
            |_, lines| fired_at(lines, guard, hostile) > 0,
        )
        .await;
    assert!(
        answered,
        "a guard returns fire on the unit that hit its owner's carrier: {}; plot {}",
        s.exchange(),
        s.plot()
    );
    let on_carrier = first(
        &s.lines,
        |kind| matches!(kind, EventKind::Impact { unit, target, .. } if *unit == hostile && *target == carrier),
    );
    let answer = first(
        &s.lines,
        |kind| matches!(kind, EventKind::Fired { unit, .. } if *unit == guard),
    );
    assert!(
        on_carrier.is_some() && on_carrier < answer,
        "its first round follows the first arrival on its owner's carrier: {on_carrier:?} then {answer:?}"
    );
    assert_eq!(
        arrived(&s.lines, hostile, guard),
        0,
        "and the raider never fired on the guard itself: {}",
        s.exchange()
    );

    // (5) The return. What the game queued for the owner while he was away, in order.
    assert!(task(&s.rig).is_some(), "the field still runs");
    let queued = pending_notices(s.rig.tw.world(), owner);
    let opened = FIELD_OPENED
        .replace("{n}", "1")
        .replace("{site}", "Ahuna Flank");
    let guarding = ADMITTED_ON
        .replace("{unit}", &guard.0.to_string())
        .replace("{posture}", Posture::Guard.label());
    let homing = UNATTENDED.replace("{unit}", &homer.0.to_string());
    // Each rig's line was queued at admission and the field's own at its first snapshot, so
    // the rigs come first. Nothing else is queued: no line for his carrier, which stays
    // where it is, and none for anybody else's unit.
    assert_eq!(
        queued,
        vec![guarding.clone(), homing.clone(), opened.clone()],
        "what is queued for the owner, each once"
    );
    let back_at = s.rig.tw.world().now();
    let read = connect(&mut s.rig, owner);
    let rows: Vec<&str> = read.lines().map(str::trim).collect();
    let row = |text: &str| {
        rows.iter()
            .position(|row| *row == text)
            .unwrap_or_else(|| panic!("the owner reads {text:?} at login: {read}"))
    };
    // He reads exactly that under the one heading, and nothing after it.
    assert_eq!(
        rows[row(NOTICE_HEADING) + 1..],
        [guarding.as_str(), homing.as_str(), opened.as_str()],
        "under the heading: each rig, then the field, and no more: {read}"
    );
    for other in [worker, hostile] {
        assert!(
            !read.contains(&format!("#{}", other.0)) && !read.contains(&name(&s.rig, other)),
            "nobody else's unit is in what he reads: {read}"
        );
    }
    assert!(
        pending_notices(s.rig.tw.world(), owner).is_empty(),
        "read once"
    );
    // He is seated again, in the running field.
    let seated = BACK_IN_SEAT
        .replace("{unit}", &guard.0.to_string())
        .replace("{station}", "pilot");
    assert!(read.contains(&seated), "{seated}: {read}");
    assert!(
        away_of(s.rig.tw.world(), guard).is_none() && bound(&s.rig, owner),
        "the owner is seated again: {:?}",
        away_of(s.rig.tw.world(), guard)
    );
    let focus = run(&mut s.rig.tw, owner, &format!("+focus #{}/pilot", guard.0));
    assert!(
        s.wait(30, |rig, _| frame(rig, owner).is_some()).await,
        "the returning pilot's cockpit frame arrives: {focus}"
    );
    assert_eq!(
        frame(&s.rig, owner).map(|frame| (frame.cell, frame.posture)),
        Some((Some(GUARD_AT), Posture::Guard)),
        "the rig is where its mind stood it, and keeps its word until he changes it"
    );
    // His first command is what the rig does: it leaves the cell its mind stood on.
    let drive = run(
        &mut s.rig.tw,
        owner,
        &format!("DRIVE {},{}", GOAL.0, GOAL.1),
    );
    let obeyed = s
        .wait(60, |rig, _| {
            frame(rig, owner).is_some_and(|frame| {
                frame.order.is_none()
                    && frame.cell.is_some_and(|at| at != GUARD_AT)
                    && frame
                        .motor
                        .as_ref()
                        .is_some_and(|motor| motor.requested_goal == GOAL)
            })
        })
        .await;
    assert!(
        obeyed,
        "the owner's DRIVE is the rig's goal, the mind's order is gone and the rig moves: \
         {drive}; {:?}",
        frame(&s.rig, owner).map(|frame| (frame.cell, frame.order, frame.motor))
    );
    assert_eq!(
        saved(&s.rig, guard).map(|(absent, _)| absent),
        Some(false),
        "the kernel no longer flies it for an absent player"
    );
    // And the mind's fire stops with its tenure. The raider fires on the carrier again, the
    // very fire the mind answered: the seat is at FIRE HOLD, where every seat starts, and in
    // its pilot's hands the rig fires nothing.
    let _ = s.wait(3, |_, _| false).await;
    let spent = fired(&s.lines, guard);
    let landed = arrived(&s.lines, hostile, carrier);
    let again = s
        .raid(
            move |row| row.designator.to_string().starts_with(&ship_again),
            90,
            |_, lines| arrived(lines, hostile, carrier) > landed,
        )
        .await;
    assert!(
        again,
        "the raider's fire arrives on the carrier again: {}; plot {}",
        s.exchange(),
        s.plot()
    );
    let _ = s.wait(6, |_, _| false).await;
    assert_eq!(
        fired(&s.lines, guard),
        spent,
        "back in its pilot's hands the rig fires only when he says: {}",
        s.exchange()
    );

    // (6) The close. Nobody fires, the field settles, and the owner, connected, is told what
    // became of each unit.
    assert!(
        s.wait(u64::from(IDLE_S) + 90, |rig, _| {
            task(rig).is_none()
                && site_record(rig).is_some_and(|record| record.open_field().is_none())
        })
        .await,
        "the field demotes"
    );
    let live_lines = said(&s.rig.tw, &s.heard, owner);
    let settled = FIELD_SETTLED
        .replace("{n}", "1")
        .replace("{site}", "Ahuna Flank");
    assert_eq!(
        count(&live_lines, &settled),
        1,
        "the settling, once: {live_lines}"
    );
    let fate_line = |fate: Fate| {
        let head = after(fate);
        live_lines
            .lines()
            .map(str::trim)
            .find(|text| text.starts_with(&head))
            .unwrap_or_else(|| panic!("a line for {:?}: {live_lines}", fate.word()))
            .to_owned()
    };
    let holding = fate_line(Fate::Holding);
    assert!(
        holding.contains(&name(&s.rig, guard)) && holding.len() < 120,
        "the guard holds on the site: {holding}"
    );
    let home = fate_line(Fate::Home);
    assert!(
        home.contains(&name(&s.rig, homer))
            && home.contains(&name(&s.rig, carrier))
            && home.len() < 120,
        "the prospector and the carrier are home: {home}"
    );
    for fate in [
        Fate::Lost,
        Fate::Captured,
        Fate::InTow,
        Fate::Down,
        Fate::Away,
    ] {
        assert!(
            !live_lines.contains(&after(fate)),
            "no unit of his met {:?}: {live_lines}",
            fate.word()
        );
    }
    assert!(
        pending_notices(s.rig.tw.world(), owner).is_empty(),
        "said, not queued"
    );

    // (7) The field's own record of fire, in order, over the whole black box.
    let _ = s.wait(2, |_, _| false).await;
    let ours = [guard, homer, carrier];
    let first_round = s.lines.iter().find_map(|line| match &line.kind {
        EventKind::Fired { unit, .. } => Some(*unit),
        _ => None,
    });
    assert_eq!(
        first_round,
        Some(hostile),
        "the first round of the field is the raider's"
    );
    let on_ours = first(
        &s.lines,
        |kind| matches!(kind, EventKind::Impact { unit, target, .. } if *unit == hostile && ours.contains(target)),
    );
    let of_ours = first(
        &s.lines,
        |kind| matches!(kind, EventKind::Fired { unit, .. } if ours.contains(unit)),
    );
    assert!(
        on_ours.is_some() && on_ours < of_ours,
        "no unit of the owner's fired before the raider's fire arrived on one: {on_ours:?} \
         then {of_ours:?}"
    );
    for unit in ours {
        assert_eq!(
            fired(&s.lines, unit),
            fired_at(&s.lines, unit, hostile),
            "#{} fired on the unit that fired, and on nobody else: {}",
            unit.0,
            s.exchange()
        );
    }
    assert!(fired(&s.lines, guard) > 0, "{}", s.exchange());
    assert_eq!(
        fired(&s.lines, homer),
        0,
        "the prospector answered for nobody on its way home"
    );

    // (8) The same words, on each unit's own record, after the demotion.
    let words = |rig: &Rig| {
        (
            held(rig, guard),
            held(rig, homer),
            held(rig, carrier),
            held(rig, worker),
        )
    };
    let left = (
        Posture::Guard,
        Posture::Return,
        Posture::Hold,
        Posture::Hold,
    );
    assert_eq!(words(&s.rig), left, "each unit holds the word it was left");
    assert_eq!(
        mush_sim::pose_of(s.rig.tw.world(), carrier).map(|(on, pose)| (on, pose.x_cm, pose.y_cm)),
        Some(pad),
        "the ship is on her pad, where she was settled"
    );
    assert!(s.aboard(homer), "with the prospector in her bay");
    assert_eq!(fired(&s.lines, carrier), 0, "and she fired nothing");
    // AP.9: what the ship did on her word goes here. Today a carrier on her pad is flown as
    // HOLD whatever her record says (doc 45 unit AP.7), so nothing above tells her HOLD from
    // a RETURN: the record is all this story can hold her to. AP.9 adds a ship in a bubble
    // that holds station and complies on the same word the rigs here obeyed.

    // (9) The casualty. The claim is the economy's contract: its window is the configured 48
    // IC days, counted from the second it was filed, while the title holder was away.
    let (casualty, claim, filed_at) = (s.casualty, s.claim, s.filed_at);
    assert_eq!(
        s.rig.cfg.recovery.claim_window_ic_s,
        WINDOW_IC_DAYS * 86_400,
        "the story left the claim's window as shipped: 48 IC days"
    );
    let window = tacsim::surface::claim::claim_window_real_secs(s.rig.tw.world());
    assert_eq!(
        window,
        WINDOW_IC_DAYS * 86_400 / i64::from(LIVE_RATE),
        "48 IC days at the live rate are 48 real hours"
    );
    assert!(
        filed_at < back_at,
        "the claim was filed before the owner came back"
    );
    let check = |rig: &Rig, when: &str| {
        let filed = contract(rig, claim);
        assert_eq!(filed.state, ContractState::Active, "{when}");
        assert_eq!(
            mush_economy::salvage_claim_hull(&filed),
            Some(casualty),
            "{when}: a salvage claim on the casualty"
        );
        assert_eq!(
            filed.effective_real_window_end(),
            Some(filed_at + window),
            "{when}: the window runs 48 IC days from the filing, not from the owner's return \
             (which would end it at {})",
            back_at + window
        );
        assert_eq!(
            title(rig, casualty),
            Some(PrincipalId::Object(owner)),
            "{when}: the title has not moved"
        );
    };
    check(&s.rig, "after the owner's return");
    // He was written to at the filing, under the site's name, while he was away.
    let letters = s.post.lock().expect("the post").clone();
    let to_owner: Vec<_> = letters
        .iter()
        .filter(|(to, _, _)| to.contains(&owner))
        .collect();
    assert_eq!(to_owner.len(), 1, "one letter, for the claim: {letters:?}");
    assert!(
        to_owner[0].2.contains(&name(&s.rig, casualty)) && to_owner[0].2.contains("48 real hours"),
        "the letter names the casualty and the window: {}",
        to_owner[0].2
    );
    // And the claim is his to answer: he reads it, and the registry still takes a contest.
    let standing = run(&mut s.rig.tw, owner, "+salvage");
    assert!(
        standing.contains(&name(&s.rig, casualty)),
        "the owner reads the claim he stands in: {standing}"
    );
    let contest = run(
        &mut s.rig.tw,
        owner,
        &format!("+salvage/contest #{}", casualty.0),
    );
    assert!(
        contest.contains("You would contest the claim"),
        "the window is open to him: {contest}"
    );

    // A restart. The words and the window are what they were.
    let Story { rig, intruder, .. } = s;
    let rig = reboot(rig, &[owner, miner, intruder]);
    assert_eq!(words(&rig), left, "each record survives the restart");
    check(&rig, "after a restart");
}

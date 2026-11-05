//! **Doc 40 cut FR.5 — the coupled tow in a running field (story a, re-run)** (Q-40-7, Q-40-25;
//! doc 36 Q-36-20), over the composed world (`driving`'s keyed rig), the arena's egress pumped by
//! hand as `field_gc4.rs` pumps it, on both grounds of `grounds_rig` (Ahuna Flank at 256, Urvara
//! Reach at 512).
//!
//! The captain's party: a Dray-P on the site's pad, its escort (rig 1) a casualty beside the
//! ramp and its wingman (rig 2) taking it in tow by `+tv/2 tow go` on the open ground. A
//! stranger's armed hull west of the pad, its pilot seated, opens a field by its mark; the tow is
//! admitted coupled, the casualty a kernel unit of its tug's party trailing it. Three stories:
//!
//! 1. **Struck, then released** (`fr5_*_the_casualty_is_struck_in_tow_and_released_in_the_field`):
//!    the stranger fires on the casualty until a hit line names it; the wingman drives, and the
//!    casualty trails; `+tv/2 release go` is a field act whose kernel answer retires the tow on
//!    the site's record; the wingman drives on and the casualty stays where it was let go, and is
//!    indexed there at the demotion.
//! 2. **The tug falls** (`fr5_*_a_tug_that_falls_drops_its_casualty_where_it_stands`): the
//!    stranger fires on the wingman until it is destroyed; the kernel uncouples, the casualty
//!    stands where it is, and the demotion indexes it at its own cell, the tow ended.
//! 3. **The coupled withdraw at the ramp** (`fr5_*_a_coupled_pair_is_withdrawn_at_the_ramp_and_loaded`):
//!    under the stranger's fire, `+tv/2 load go` away from the ramp is refused with the tow's
//!    not-at-ramp sentence; the wingman drives the casualty to the ramp, `+tv/2 load go` asks the
//!    field for both, both are checked in through the ordinary exit, and the load runs as on open
//!    ground into the carrier's bay while the field still runs.
//!
//! **Doc 40, the ramp exit's boarding window, player hulls** (the owner's ruling, 2026-11-04,
//! "back in the field"): a recovery or a load counts only once the hull is aboard.
//!
//! 4. **The carrier lost inside the load's window**
//!    (`fr5_ahuna_a_carrier_lost_before_the_load_keeps_the_pair_in_the_field`): the pair is let
//!    go at the ramp, its exits held on their way to the site record (the K3 lane's injected
//!    fault, `field_crash`'s), and the stranger's fire takes the carrier's hull. The pair goes
//!    back into the field coupled, its pilot still flying the tug, and nothing is loaded.
//! 5. **The carrier lost inside the recover's window**
//!    (`fr5_ahuna_a_carrier_lost_before_the_recover_keeps_the_rig_in_the_field`): the same for a
//!    rig's `+tv/1 recover go`.
//! 6. **The carrier lost before the release**
//!    (`fr5_ahuna_a_rig_whose_carrier_is_lost_is_kept_in_the_field`): the kernel keeps the rig at
//!    the ramp, its pilot's.
//!
//! **What is a fixture.** The escort's power, destroyed by staff hand, and its row on the site's
//! index (GC.5's fixture, which the field that disabled it would have written; `field_rs5`'s
//! precedent), standing in for the first field of `field_gc4`'s story; the wingman's armour at
//! its floor for the fall (`fragile`). Every act after is the production command path.

use mush_core::ObjId;
use tacsim::arena::ledger::{EventKind, LedgerEvent};
use tacsim::arena_task::ArenaOut;
use tokio::sync::mpsc::Receiver;

use crate::driving_rig::{
    Rig, act_go, cid, command, drain, finish, land, launched_at, on_pad_as_at, ramp_at, run, said,
    staff_hand, stocked_tuned, to_cell_on, warm,
};
use crate::field_host_rig::{
    armed_on, egress, fragile, frame, seat, second, site_record_at, task, tune, weapon_of,
};
use crate::grounds_rig::{AHUNA_FLANK, Ground, URVARA_REACH};

/// GC.4's stage timings, and a field that demotes `idle` seconds after its last shot.
fn quick(cfg: &mut tacsim::TacsimConfig, idle: u32) {
    tune(cfg);
    cfg.field.disengage_s = 3_600;
    cfg.field.idle_demote_s = idle;
    cfg.recovery.inspect_s = 30;
    cfg.recovery.attach_s = 30;
    cfg.recovery.load_s = 60;
    // Position summaries every tenth of a second fill a ledger chunk in seconds, so the field's
    // hit lines reach the host while it runs (`field_fr4`'s knob).
    cfg.arena.ledger.summary_ms = 100;
}

/// The escort's power destroyed (`field_gc4`'s `powerless`): a casualty a tug can take.
fn powerless(rig: &mut Rig, unit: ObjId) {
    staff_hand(rig, |ctx| {
        let plan = mush_tactics::UnitMutationPlan::new(
            vec![mush_tactics::UnitMutationOp::SetComponentCondition {
                component: mush_tactics::unit::ComponentId::new("power").expect("an id"),
                condition: mush_tactics::ConditionId::new("destroyed").expect("a condition"),
            }],
            None,
            Some(smol_str::SmolStr::new_static("test:fr5-powerless")),
        )
        .expect("a plan");
        mush_tactics::module::apply_plan(ctx, unit, &plan).expect("applied");
    });
}

/// `unit`'s site cell on the surface, when it stands on the ground.
fn cell_of(rig: &Rig, unit: ObjId) -> Option<(i64, i64)> {
    mush_sim::pose_of(rig.tw.world(), unit)
        .map(|(_, at)| (at.x_cm.div_euclid(1_000), at.y_cm.div_euclid(1_000)))
}

/// Where the running field last stood `unit`, and the tug it trails (its last snapshot).
fn stand(rig: &Rig, ground: &Ground, unit: ObjId) -> Option<((i64, i64), Option<ObjId>)> {
    tacsim::surface::tow::field_stand(rig.tw.world(), &cid(ground.entry), unit)
}

/// The tug's tow, as the site's record holds it.
fn towing(rig: &mut Rig, ground: &Ground, tug: ObjId) -> Option<ObjId> {
    site_record_at(rig, ground.entry)
        .and_then(|record| record.tows.towing(tug).map(|op| op.casualty()))
}

/// Whether the field has demoted.
fn demoted(rig: &mut Rig, ground: &Ground) -> bool {
    task(rig).is_none()
        && site_record_at(rig, ground.entry).is_some_and(|record| record.open_field().is_none())
}

/// Chebyshev cells apart.
fn apart(a: (i64, i64), b: (i64, i64)) -> i64 {
    (a.0 - b.0).abs().max((a.1 - b.1).abs())
}

/// **The listeners' ears**: what `who` heard from the field host's answers, in order.
struct Ears {
    who: Vec<ObjId>,
    heard: Vec<String>,
    /// The host's `op` writes held off the site record while set (`field_crash`'s injected
    /// fault): the exits, to keep a boarding window open.
    hold: Option<&'static str>,
}

impl Ears {
    fn new(who: &[ObjId]) -> Self {
        Self {
            who: who.to_vec(),
            heard: vec![String::new(); who.len()],
            hold: None,
        }
    }

    fn of(&self, who: ObjId) -> &str {
        self.who
            .iter()
            .position(|w| *w == who)
            .and_then(|at| self.heard.get(at))
            .map_or("", String::as_str)
    }

    fn take(&mut self, events: &[(mush_core::Audience, mush_proto::OutputEvent)], rig: &Rig) {
        for (who, heard) in self.who.iter().zip(self.heard.iter_mut()) {
            let line = said(&rig.tw, events, *who);
            if !line.is_empty() {
                heard.push_str(&line);
                heard.push('\n');
            }
        }
    }
}

/// **`field_host_rig::pump`, keeping what the listeners heard** from the bridge's answers.
fn pump(rig: &mut Rig, rx: &mut Receiver<ArenaOut>, lines: &mut Vec<LedgerEvent>, ears: &mut Ears) {
    let Some(tasks) = rig
        .tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .map(|registry| registry.tasks().clone())
    else {
        return;
    };
    let mut staged = 0;
    while let Ok(message) = rx.try_recv() {
        if let ArenaOut::Ledger { chunk, .. } = &message
            && let Ok(text) = std::str::from_utf8(&chunk.bytes)
        {
            lines.extend(
                text.lines()
                    .filter_map(|line| serde_json::from_str::<LedgerEvent>(line).ok()),
            );
        }
        if !tasks.stage(message) {
            break;
        }
        staged += 1;
    }
    if staged > 0 {
        let events = rig.tw.apply(mush_core::WorldCommand::ModuleAsyncDone {
            module: smol_str::SmolStr::new_static(tacsim::arena_bridge::MODULE),
            actor: mush_core::GOD,
            correlation: 0,
            payload: tacsim::arena_bridge::EGRESS_PAYLOAD.to_owned(),
        });
        ears.take(&events, rig);
        tasks.stamp_ledger_writes(1);
        tasks.ack_ledger_upto(1);
    }
}

/// Pump and tick until `done` holds or `secs` of wall time pass (`field_host_rig::until`, with
/// the listeners' ears); answers whether it held. Every tick's submission is landed, and none may
/// be a refused tow act.
async fn until(
    rig: &mut Rig,
    (rx, lines, ears): (&mut Receiver<ArenaOut>, &mut Vec<LedgerEvent>, &mut Ears),
    secs: u64,
    mut done: impl FnMut(&mut Rig, &[LedgerEvent], &Ears) -> bool,
) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    let mut beat = 0_u32;
    while std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        pump(rig, rx, lines, ears);
        beat += 1;
        if beat.is_multiple_of(8) {
            let landed = match ears.hold {
                Some(op) => second_holding(rig, op),
                None => second(rig),
            };
            let refused: Vec<String> = landed
                .into_iter()
                .filter(|error| error.contains(tacsim::surface::tow::TOW_ACTION))
                .collect();
            assert!(refused.is_empty(), "a tow landing refused: {refused:?}");
        }
        if done(rig, lines, ears) {
            return true;
        }
    }
    false
}

/// **One world second with the host's `op` writes held off the site record** (`field_crash`'s
/// `second_dropping`): the ticks due and every other submission landed; answers the landings'
/// errors.
fn second_holding(rig: &mut Rig, op: &str) -> Vec<String> {
    let t = rig.tw.world().now().saturating_add(1);
    rig.tw.set_now(t);
    let _ = rig.tw.fire_due(t.saturating_mul(1_000));
    let mut errors = Vec::new();
    for submit in drain(&mut rig.tw) {
        let held = submit.action.key().id.as_str() == tacsim::field::FIELD_ACTION
            && submit.parameters.get("op").and_then(|op| op.as_str()) == Some(op);
        if !held && let Err(error) = land(rig, &submit) {
            errors.push(error);
        }
    }
    errors
}

/// **The ground every story stands on** (this module's head): the captain's party with the
/// escort a casualty at `casualty_at` and the wingman at `tug_at` (offsets from the ramp's cell),
/// the stranger's seated armed hull west of the pad, and the tow attached on the open ground.
struct Set {
    a: ObjId,
    b: ObjId,
    carrier: ObjId,
    casualty: ObjId,
    tug: ObjId,
    hound: ObjId,
    ramp: (i64, i64),
}

fn set_on(
    ground: &Ground,
    (tug_at, casualty_at): ((i64, i64), (i64, i64)),
    idle: u32,
) -> (Rig, Set) {
    let mut rig = stocked_tuned(|cfg| quick(cfg, idle));
    let (a, row, tvs) = on_pad_as_at(&mut rig, ("Pilot", 1, "Salt Moth"), ground.site(), 2);
    let carrier = ObjId(row.hull);
    let (casualty, tug) = (tvs[0], tvs[1]);
    launched_at(&mut rig, ground.entry, a, casualty);
    let _ = act_go(&mut rig, a, "+tv/2 launch go");
    let _ = finish(&mut rig, tug, a);
    let (rx, ry, _) = ramp_at(&rig.tw, ground.entry);
    let ramp = (rx.div_euclid(1_000), ry.div_euclid(1_000));
    let casualty_cell = (ramp.0 + casualty_at.0, ramp.1 + casualty_at.1);
    to_cell_on(&mut rig, ground.entry, casualty, casualty_cell);
    to_cell_on(
        &mut rig,
        ground.entry,
        tug,
        (ramp.0 + tug_at.0, ramp.1 + tug_at.1),
    );
    powerless(&mut rig, casualty);
    // The field that disabled it would have indexed it (GC.5's fixture).
    let staff = crate::site_record_rig::staff_member(&mut rig);
    crate::site_record_rig::staff(
        &mut rig,
        staff,
        &format!(
            "@tacsim/site {} leave #{} casualty {},{}",
            ground.entry, casualty.0, casualty_cell.0, casualty_cell.1
        ),
    );
    // The stranger: its own Dray-P on the pad, its armed hull west of it, seated.
    let (b, _, _) = on_pad_as_at(&mut rig, ("Second", 2, "Red Moth"), ground.site(), 1);
    let hound = armed_on(
        &mut rig,
        ground.entry,
        "Red Hound",
        (ramp.0 - 8, ramp.1),
        90_000,
    );
    seat(&mut rig, hound, b);
    seat(&mut rig, tug, a);
    warm(&mut rig);
    // The tow on the open ground (GC.4).
    let _ = second(&mut rig);
    let (out, queued) = command(&mut rig, a, "+tv/2 tow go");
    assert_eq!(queued.len(), 1, "one tow start: {out}");
    land(&mut rig, &queued[0]).expect("the tow start lands");
    let mut attached = false;
    for _ in 0..120 {
        if towing(&mut rig, ground, tug) == Some(casualty) {
            attached = true;
            break;
        }
        let _ = second(&mut rig);
    }
    assert!(attached, "the tow attaches");
    (
        rig,
        Set {
            a,
            b,
            carrier,
            casualty,
            tug,
            hound,
            ramp,
        },
    )
}

/// **The stranger's mark opens the field**; the tug and its casualty are both in it.
async fn open(
    rig: &mut Rig,
    ground: &Ground,
    set: &Set,
    io: (&mut Receiver<ArenaOut>, &mut Vec<LedgerEvent>, &mut Ears),
) {
    let (out, queued) = command(rig, set.b, "+field/mark E=hostile");
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    land(rig, &queued[0]).expect("the promotion lands");
    assert!(
        until(rig, io, 20, |rig, _, _| task(rig).is_some()
            && stand(rig, ground, set.casualty).is_some())
        .await,
        "the field opens and snapshots"
    );
    let site = tacsim::field::field_of_unit(rig.tw.world(), set.tug);
    assert_eq!(site, Some(cid(ground.entry)), "the tug is in the field");
    assert_eq!(
        tacsim::field::field_of_unit(rig.tw.world(), set.casualty),
        site,
        "the casualty is in the field with its tug (Q-40-7)"
    );
}

/// **The stranger fires on `target`, standing at `cell`, until `done` reads the ledger.**
async fn fire_on(
    rig: &mut Rig,
    set: &Set,
    (rx, lines, ears): (&mut Receiver<ArenaOut>, &mut Vec<LedgerEvent>, &mut Ears),
    cell: (i64, i64),
    done: impl Fn(&[LedgerEvent]) -> bool,
) -> bool {
    let gun = weapon_of(rig, set.hound);
    let b = set.b;
    let mut said_last = String::new();
    let held = until(
        rig,
        (&mut *rx, &mut *lines, &mut *ears),
        90,
        |rig, lines, _| {
            if done(lines) {
                return true;
            }
            let contact = frame(rig, b).and_then(|frame| {
                frame
                    .contacts
                    .iter()
                    .find(|contact| {
                        contact
                            .cell
                            .is_some_and(|(x, y)| (i64::from(x), i64::from(y)) == cell)
                    })
                    .cloned()
            });
            if let Some(contact) = contact {
                if frame(rig, b).is_some_and(|frame| frame.lock != Some(contact.id)) {
                    said_last = run(&mut rig.tw, b, &format!("LOCK {}", contact.designator));
                }
                said_last.push_str(" | ");
                said_last.push_str(&run(&mut rig.tw, b, &format!("FIRE {gun}")));
            }
            false
        },
    )
    .await;
    if !held {
        let contacts: Vec<String> = frame(rig, b)
            .map(|frame| {
                frame
                    .contacts
                    .iter()
                    .map(|c| format!("{} {:?} {:?}", c.designator, c.cell, c.relation))
                    .collect()
            })
            .unwrap_or_default();
        let shots: Vec<String> = lines
            .iter()
            .filter(|line| {
                matches!(
                    line.kind,
                    EventKind::Fired { .. } | EventKind::Hit { .. } | EventKind::Miss { .. }
                )
            })
            .map(|line| format!("{:?}", line.kind))
            .take(12)
            .collect();
        eprintln!(
            "fire_on {cell:?} failed: contacts {contacts:?}; last said {said_last}; shots {shots:?}"
        );
    }
    held
}

/// Whether a hit line names `target`.
fn struck(lines: &[LedgerEvent], target: ObjId) -> bool {
    lines
        .iter()
        .any(|line| matches!(line.kind, EventKind::Hit { target: hit, .. } if hit == target))
}

// ---- 1. struck, then released --------------------------------------------------------------

#[allow(clippy::too_many_lines)] // one story, in order
async fn released_on(ground: &Ground) {
    // The wingman east of the casualty, so its drive east drags the casualty behind it.
    let (mut rig, set) = set_on(ground, ((5, 0), (4, 0)), 90);
    let (a, casualty, tug) = (set.a, set.casualty, set.tug);
    let mut rx = egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();
    let mut ears = Ears::new(&[a]);
    open(&mut rig, ground, &set, (&mut rx, &mut lines, &mut ears)).await;
    assert_eq!(
        stand(&rig, ground, casualty).and_then(|(_, tug)| tug),
        Some(tug),
        "the casualty is coupled to its tug"
    );

    // Struck while towed: a hit lands on the casualty by its own geometry.
    let at = stand(&rig, ground, casualty)
        .map(|(cell, _)| cell)
        .expect("its stand");
    assert!(
        fire_on(
            &mut rig,
            &set,
            (&mut rx, &mut lines, &mut ears),
            at,
            |lines| struck(lines, casualty)
        )
        .await,
        "the stranger's fire strikes the casualty in tow"
    );

    // The wingman drives east; the casualty trails on the cells it leaves.
    let goal = (set.ramp.0 + 8, set.ramp.1);
    let drive = run(&mut rig.tw, a, &format!("DRIVE {},{}", goal.0, goal.1));
    assert!(!drive.contains("cannot"), "{drive}");
    assert!(
        until(
            &mut rig,
            (&mut rx, &mut lines, &mut ears),
            60,
            |rig, _, _| { stand(rig, ground, tug).is_some_and(|(cell, _)| cell.0 >= goal.0 - 1) }
        )
        .await,
        "the wingman drives east: {:?}",
        stand(&rig, ground, tug)
    );
    let (tug_cell, _) = stand(&rig, ground, tug).expect("the tug's stand");
    let (trail, coupled) = stand(&rig, ground, casualty).expect("the casualty's stand");
    assert_eq!(coupled, Some(tug), "still coupled");
    assert_ne!(trail, at, "the casualty moved with its tug");
    assert_eq!(
        apart(trail, tug_cell),
        1,
        "it trails its tug: {trail:?}, tug {tug_cell:?}"
    );

    // `+tv/2 release` inside the field is a field act (Q-40-7).
    let preview = run(&mut rig.tw, a, "+tv/2 release");
    assert!(preview.contains("inside field"), "{preview}");
    let (out, queued) = command(&mut rig, a, "+tv/2 release go");
    assert!(queued.is_empty(), "a field act, not a surface act: {out}");
    assert!(out.contains("at field"), "the release is ordered: {out}");
    assert!(
        until(
            &mut rig,
            (&mut rx, &mut lines, &mut ears),
            20,
            |rig, _, ears| {
                ears.of(a).contains("lies where it stands") && towing(rig, ground, tug).is_none()
            }
        )
        .await,
        "the kernel's Released answer took and the record's tow is retired: {}",
        ears.of(a)
    );
    assert!(task(&rig).is_some(), "inside the running field");
    assert!(
        until(
            &mut rig,
            (&mut rx, &mut lines, &mut ears),
            10,
            |rig, _, _| {
                stand(rig, ground, casualty).is_some_and(|(_, coupled)| coupled.is_none())
            }
        )
        .await,
        "the casualty no longer trails"
    );
    let (released_at, _) = stand(&rig, ground, casualty).expect("the casualty's stand");

    // The wingman drives on alone; the casualty stays where it was let go.
    let on = (set.ramp.0 + 11, set.ramp.1);
    let drive = run(&mut rig.tw, a, &format!("DRIVE {},{}", on.0, on.1));
    assert!(!drive.contains("cannot"), "{drive}");
    assert!(
        until(
            &mut rig,
            (&mut rx, &mut lines, &mut ears),
            60,
            |rig, _, _| { stand(rig, ground, tug).is_some_and(|(cell, _)| cell.0 >= on.0 - 1) }
        )
        .await,
        "the wingman drives on"
    );
    assert_eq!(
        stand(&rig, ground, casualty).map(|(cell, _)| cell),
        Some(released_at),
        "the casualty does not follow"
    );

    // The field settles; the casualty is indexed where it was let go, no longer in tow.
    assert!(
        until(
            &mut rig,
            (&mut rx, &mut lines, &mut ears),
            120,
            |rig, _, _| demoted(rig, ground)
        )
        .await,
        "the field settles"
    );
    assert_eq!(
        cell_of(&rig, casualty),
        Some(released_at),
        "where it was let go"
    );
    let record = site_record_at(&mut rig, ground.entry).expect("the site record");
    assert!(record.tows.towing(tug).is_none(), "no tow");
    let row = record
        .units
        .iter()
        .find(|left| left.unit == casualty)
        .unwrap_or_else(|| panic!("the casualty is indexed: {:?}", record.units));
    assert_eq!(row.pose.cell(), released_at, "indexed where it stands");
}

/// **FR.5 at 256**: Ahuna Flank.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fr5_ahuna_the_casualty_is_struck_in_tow_and_released_in_the_field() {
    released_on(&AHUNA_FLANK).await;
}

/// **FR.5 at 512**: Urvara Reach.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fr5_urvara_the_casualty_is_struck_in_tow_and_released_in_the_field() {
    released_on(&URVARA_REACH).await;
}

// ---- 2. the tug falls ----------------------------------------------------------------------

async fn fallen_on(ground: &Ground) {
    let (mut rig, set) = set_on(ground, ((4, 0), (3, 0)), 30);
    let (a, casualty, tug) = (set.a, set.casualty, set.tug);
    fragile(&mut rig, tug);
    let mut rx = egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();
    let mut ears = Ears::new(&[a]);
    open(&mut rig, ground, &set, (&mut rx, &mut lines, &mut ears)).await;
    let (stands, coupled) = stand(&rig, ground, casualty).expect("the casualty's stand");
    assert_eq!(coupled, Some(tug), "coupled");
    let (tug_cell, _) = stand(&rig, ground, tug).expect("the tug's stand");

    // The stranger's fire takes the wingman.
    assert!(
        fire_on(
            &mut rig,
            &set,
            (&mut rx, &mut lines, &mut ears),
            tug_cell,
            |lines| lines.iter().any(
                |line| matches!(line.kind, EventKind::UnitDestroyed { unit, .. } if unit == tug)
            )
        )
        .await,
        "the stranger's fire destroys the tug"
    );
    // The kernel drops the tow with its tug (`arena::tow`'s own gate), and the casualty stands
    // where it was. The field settles; the casualty is indexed at its own cell, the tow ended.
    assert!(
        until(
            &mut rig,
            (&mut rx, &mut lines, &mut ears),
            120,
            |rig, _, _| demoted(rig, ground)
        )
        .await,
        "the field settles"
    );
    assert_eq!(
        cell_of(&rig, casualty),
        Some(stands),
        "dropped where it stood"
    );
    let record = site_record_at(&mut rig, ground.entry).expect("the site record");
    assert!(
        record.tows.towing(tug).is_none(),
        "the tow ended with its tug"
    );
    let row = record
        .units
        .iter()
        .find(|left| left.unit == casualty)
        .unwrap_or_else(|| panic!("the casualty keeps its row: {:?}", record.units));
    assert_eq!(row.pose.cell(), stands, "indexed where it stands");
    assert_ne!(Some(stands), cell_of(&rig, tug), "never on its tug's cell");
}

/// **FR.5 at 256**: Ahuna Flank.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fr5_ahuna_a_tug_that_falls_drops_its_casualty_where_it_stands() {
    fallen_on(&AHUNA_FLANK).await;
}

/// **FR.5 at 512**: Urvara Reach.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fr5_urvara_a_tug_that_falls_drops_its_casualty_where_it_stands() {
    fallen_on(&URVARA_REACH).await;
}

// ---- 3. the coupled withdraw at the ramp ---------------------------------------------------

#[allow(clippy::too_many_lines)] // one story, in order
async fn withdrawn_on(ground: &Ground) {
    // The wingman west of the casualty, so its drive west to the ramp drags the casualty behind.
    let (mut rig, set) = set_on(ground, ((3, 0), (4, 0)), 300);
    let (a, casualty, tug, carrier) = (set.a, set.casualty, set.tug, set.carrier);
    let mut rx = egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();
    let mut ears = Ears::new(&[a]);
    open(&mut rig, ground, &set, (&mut rx, &mut lines, &mut ears)).await;

    // Under fire: the stranger strikes the casualty.
    let at = stand(&rig, ground, casualty)
        .map(|(cell, _)| cell)
        .expect("its stand");
    assert!(
        fire_on(
            &mut rig,
            &set,
            (&mut rx, &mut lines, &mut ears),
            at,
            |lines| struck(lines, casualty)
        )
        .await,
        "the stranger's fire strikes the casualty in tow"
    );
    // A tug in tow still never recovers (Q-40-25).
    let refused = run(&mut rig.tw, a, "+tv/2 recover go");
    assert!(refused.contains("has a casualty in tow"), "{refused}");

    // Away from the ramp the kernel refuses the pair, in the tow's own words.
    let (out, queued) = command(&mut rig, a, "+tv/2 load go");
    assert!(queued.is_empty(), "a field act: {out}");
    assert!(
        until(
            &mut rig,
            (&mut rx, &mut lines, &mut ears),
            20,
            |_, _, ears| {
                ears.of(a)
                    .contains("is not within a cell of a carrier's ramp")
            }
        )
        .await,
        "refused away from the ramp: {}",
        ears.of(a)
    );
    assert!(
        tacsim::field::field_of_unit(rig.tw.world(), tug).is_some()
            && tacsim::field::field_of_unit(rig.tw.world(), casualty).is_some(),
        "both fight on"
    );

    // The wingman drags the casualty to the ramp.
    let goal = set.ramp;
    let drive = run(&mut rig.tw, a, &format!("DRIVE {},{}", goal.0, goal.1));
    assert!(!drive.contains("cannot"), "{drive}");
    assert!(
        until(
            &mut rig,
            (&mut rx, &mut lines, &mut ears),
            60,
            |rig, _, _| {
                stand(rig, ground, tug).is_some_and(|(cell, _)| apart(cell, set.ramp) <= 1)
            }
        )
        .await,
        "the wingman reaches the ramp: {:?}",
        stand(&rig, ground, tug)
    );
    let (tug_cell, _) = stand(&rig, ground, tug).expect("the tug's stand");
    let (trail, coupled) = stand(&rig, ground, casualty).expect("the casualty's stand");
    assert_eq!(coupled, Some(tug), "still coupled");
    assert_eq!(
        apart(trail, tug_cell),
        1,
        "it trails: {trail:?}, tug {tug_cell:?}"
    );

    // The coupled withdraw (Q-40-25): both out through the ordinary exit, then the load.
    let preview = run(&mut rig.tw, a, "+tv/2 load");
    assert!(
        preview.contains("out of field") && preview.contains("Salt Moth"),
        "{preview}"
    );
    let (out, queued) = command(&mut rig, a, "+tv/2 load go");
    assert!(queued.is_empty(), "a field act: {out}");
    assert!(out.contains("makes for the ramp"), "{out}");
    assert!(
        until(
            &mut rig,
            (&mut rx, &mut lines, &mut ears),
            120,
            |rig, _, _| {
                tacsim::surface::tow::in_bay_of(rig.tw.world(), casualty, carrier)
                    && site_record_at(rig, ground.entry)
                        .is_some_and(|record| record.tows.act_of(tug).is_none())
            }
        )
        .await,
        "the casualty is loaded into the bay: {}",
        ears.of(a)
    );
    assert!(task(&rig).is_some(), "the field still runs");
    let record = site_record_at(&mut rig, ground.entry).expect("the site record");
    assert!(record.open_field().is_some(), "loaded under the open field");
    assert!(
        record.exited(tug) && record.exited(casualty),
        "both left at the ramp"
    );
    assert!(record.tows.towing(tug).is_none(), "the tow is spent");
    assert!(
        record.units.iter().all(|left| left.unit != casualty),
        "off the index"
    );
    assert!(
        tacsim::field::field_of_unit(rig.tw.world(), tug).is_none()
            && tacsim::field::field_of_unit(rig.tw.world(), casualty).is_none(),
        "neither is the field's"
    );
    assert!(
        cell_of(&rig, tug).is_some_and(|cell| apart(cell, set.ramp) <= 1),
        "the wingman stands beside the ramp, out of the field"
    );
}

/// **FR.5 at 256**: Ahuna Flank.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fr5_ahuna_a_coupled_pair_is_withdrawn_at_the_ramp_and_loaded() {
    withdrawn_on(&AHUNA_FLANK).await;
}

/// **FR.5 at 512**: Urvara Reach.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fr5_urvara_a_coupled_pair_is_withdrawn_at_the_ramp_and_loaded() {
    withdrawn_on(&URVARA_REACH).await;
}

// ---- 4-6. the boarding window (doc 40, player hulls) ---------------------------------------

/// Whether a ledger line names `unit` destroyed.
fn destroyed(lines: &[LedgerEvent], unit: ObjId) -> bool {
    lines.iter().any(
        |line| matches!(line.kind, EventKind::UnitDestroyed { unit: fell, .. } if fell == unit),
    )
}

/// Whether `site`'s field binds `pilot` to its task: the pilot still flies a unit there.
fn flies(rig: &Rig, pilot: ObjId) -> bool {
    task(rig).is_some_and(|task| {
        rig.tw
            .world()
            .module_state::<tacsim::arena_bridge::ArenaRegistry>()
            .is_some_and(|registry| registry.pilots_of(task).contains(&pilot))
    })
}

/// Whether the field holds `unit`.
fn held(rig: &Rig, unit: ObjId) -> bool {
    tacsim::field::field_of_unit(rig.tw.world(), unit).is_some()
}

/// **The stranger takes the carrier's hull** (the carrier made `fragile` before the field).
async fn take_the_carrier(
    rig: &mut Rig,
    ground: &Ground,
    set: &Set,
    (rx, lines, ears): (&mut Receiver<ArenaOut>, &mut Vec<LedgerEvent>, &mut Ears),
) {
    let at = stand(rig, ground, set.carrier)
        .map(|(cell, _)| cell)
        .expect("the carrier's stand");
    let carrier = set.carrier;
    assert!(
        fire_on(rig, set, (rx, lines, ears), at, |lines| destroyed(
            lines, carrier
        ))
        .await,
        "the stranger's fire takes the carrier's hull"
    );
}

/// **A rig of the captain's beside its carrier's ramp**, the stranger's seated hull west of it,
/// the carrier fragile: story 5's and 6's ground. The [`Set`]'s casualty and tug are the rig.
fn rig_at_the_ramp(ground: &Ground) -> (Rig, Set) {
    let mut rig = stocked_tuned(|cfg| quick(cfg, 300));
    let (a, row, tvs) = on_pad_as_at(&mut rig, ("Pilot", 1, "Salt Moth"), ground.site(), 1);
    let carrier = ObjId(row.hull);
    let tv = tvs[0];
    launched_at(&mut rig, ground.entry, a, tv);
    let (rx, ry, _) = ramp_at(&rig.tw, ground.entry);
    let ramp = (rx.div_euclid(1_000), ry.div_euclid(1_000));
    let (b, _, _) = on_pad_as_at(&mut rig, ("Second", 2, "Red Moth"), ground.site(), 1);
    let hound = armed_on(
        &mut rig,
        ground.entry,
        "Red Hound",
        (ramp.0 - 8, ramp.1),
        90_000,
    );
    seat(&mut rig, hound, b);
    seat(&mut rig, tv, a);
    fragile(&mut rig, carrier);
    warm(&mut rig);
    (
        rig,
        Set {
            a,
            b,
            carrier,
            casualty: tv,
            tug: tv,
            hound,
            ramp,
        },
    )
}

/// **Story 4**: the pair let go at the ramp, its carrier lost before the load lands.
#[allow(clippy::too_many_lines)] // one story, in order
async fn lost_in_the_load_window_on(ground: &Ground) {
    // The wingman beside the ramp, the casualty trailing it.
    let (mut rig, set) = set_on(ground, ((1, 0), (2, 0)), 300);
    let (a, casualty, tug, carrier) = (set.a, set.casualty, set.tug, set.carrier);
    fragile(&mut rig, carrier);
    let mut rx = egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();
    let mut ears = Ears::new(&[a]);
    open(&mut rig, ground, &set, (&mut rx, &mut lines, &mut ears)).await;

    // The coupled withdraw, its exits held: the pair is out and the load has not run.
    ears.hold = Some("exit");
    let (out, queued) = command(&mut rig, a, "+tv/2 load go");
    assert!(queued.is_empty(), "a field act: {out}");
    assert!(out.contains("makes for the ramp"), "{out}");
    assert!(
        until(
            &mut rig,
            (&mut rx, &mut lines, &mut ears),
            30,
            |rig, _, _| !held(rig, tug) && !held(rig, casualty)
        )
        .await,
        "the pair is let go at the ramp: {}",
        ears.of(a)
    );
    take_the_carrier(&mut rig, ground, &set, (&mut rx, &mut lines, &mut ears)).await;

    // The exits land again; the pair is back in the field, coupled, and nothing boards.
    ears.hold = None;
    let back = until(
        &mut rig,
        (&mut rx, &mut lines, &mut ears),
        40,
        |rig, _, _| {
            held(rig, tug)
                && held(rig, casualty)
                && stand(rig, ground, casualty).is_some_and(|(_, towed)| towed == Some(tug))
        },
    )
    .await;
    assert!(
        back,
        "the pair is back in the field, coupled: tug held {}, casualty held {}, casualty stands {:?}, in the bay {}; heard {}",
        held(&rig, tug),
        held(&rig, casualty),
        stand(&rig, ground, casualty),
        tacsim::surface::tow::in_bay_of(rig.tw.world(), casualty, carrier),
        ears.of(a)
    );
    assert!(
        !tacsim::surface::tow::in_bay_of(rig.tw.world(), casualty, carrier),
        "nothing is loaded into the wreck"
    );
    assert!(flies(&rig, a), "the captain still flies the wingman");
}

/// **Story 4 at 256**: Ahuna Flank.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fr5_ahuna_a_carrier_lost_before_the_load_keeps_the_pair_in_the_field() {
    lost_in_the_load_window_on(&AHUNA_FLANK).await;
}

/// **Story 5**: the rig let go at the ramp, its carrier lost before the recover lands.
async fn lost_in_the_recover_window_on(ground: &Ground) {
    let (mut rig, set) = rig_at_the_ramp(ground);
    let (a, tv, carrier) = (set.a, set.tug, set.carrier);
    let mut rx = egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();
    let mut ears = Ears::new(&[a]);
    open(&mut rig, ground, &set, (&mut rx, &mut lines, &mut ears)).await;

    ears.hold = Some("exit");
    let asked = run(&mut rig.tw, a, "+tv/1 recover go");
    assert!(asked.contains("makes for the ramp"), "{asked}");
    assert!(
        until(
            &mut rig,
            (&mut rx, &mut lines, &mut ears),
            30,
            |rig, _, _| !held(rig, tv)
        )
        .await,
        "the rig is let go at the ramp: {}",
        ears.of(a)
    );
    take_the_carrier(&mut rig, ground, &set, (&mut rx, &mut lines, &mut ears)).await;

    ears.hold = None;
    let back = until(
        &mut rig,
        (&mut rx, &mut lines, &mut ears),
        40,
        |rig, _, _| held(rig, tv) && flies(rig, a),
    )
    .await;
    assert!(
        back,
        "the rig is back in the field, its pilot's: held {}, flies {}, in the bay {}; heard {}",
        held(&rig, tv),
        flies(&rig, a),
        tacsim::surface::tow::in_bay_of(rig.tw.world(), tv, carrier),
        ears.of(a)
    );
    assert!(
        !tacsim::surface::tow::in_bay_of(rig.tw.world(), tv, carrier),
        "it never boards the wreck"
    );
}

/// **Story 5 at 256**: Ahuna Flank.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fr5_ahuna_a_carrier_lost_before_the_recover_keeps_the_rig_in_the_field() {
    lost_in_the_recover_window_on(&AHUNA_FLANK).await;
}

/// **Story 6**: the carrier lost before the recover is asked; the kernel keeps the rig.
async fn lost_before_the_release_on(ground: &Ground) {
    let (mut rig, set) = rig_at_the_ramp(ground);
    let (a, tv, carrier) = (set.a, set.tug, set.carrier);
    let mut rx = egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();
    let mut ears = Ears::new(&[a]);
    open(&mut rig, ground, &set, (&mut rx, &mut lines, &mut ears)).await;
    take_the_carrier(&mut rig, ground, &set, (&mut rx, &mut lines, &mut ears)).await;

    let asked = run(&mut rig.tw, a, "+tv/1 recover go");
    assert!(asked.contains("makes for the ramp"), "{asked}");
    let kept = until(
        &mut rig,
        (&mut rx, &mut lines, &mut ears),
        20,
        |_, _, ears| ears.of(a).contains("carrier is lost"),
    )
    .await;
    // A few more seconds: nobody is let go late.
    let _ = until(&mut rig, (&mut rx, &mut lines, &mut ears), 3, |_, _, _| {
        false
    })
    .await;
    assert!(
        kept && held(&rig, tv) && flies(&rig, a),
        "the kernel keeps the rig, its pilot's: told {kept}, held {}, flies {}, in the bay {}; heard {}",
        held(&rig, tv),
        flies(&rig, a),
        tacsim::surface::tow::in_bay_of(rig.tw.world(), tv, carrier),
        ears.of(a)
    );
}

/// **Story 6 at 256**: Ahuna Flank.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fr5_ahuna_a_rig_whose_carrier_is_lost_is_kept_in_the_field() {
    lost_before_the_release_on(&AHUNA_FLANK).await;
}

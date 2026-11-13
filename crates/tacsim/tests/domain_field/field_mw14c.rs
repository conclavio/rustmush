//! **Doc 44 unit MW.14c — the compiled commander's gaps, and the focus-fire order** (Q-44-49;
//! doc 27 FO-15), over the composed world (`driving`'s keyed rig) with the arena's egress
//! pumped by hand, each roster flown by `roster_rig` into a field of three parties: the roster,
//! an escort of another captain's and an intruder of a third's.
//!
//! 1. **A posture stands for the dwell, a loss withdraws at once, and the organiser reads it
//!    all** ([`mw14c_a_posture_stands_for_the_dwell_and_a_loss_withdraws_at_once`]). An
//!    NPC-led roster advances. The escort fires on its carrier: a Hostile is in the
//!    commander's sight and she orders HOLD. The intruder's player destroys the escort, which
//!    leaves her sight: inside the dwell she orders nothing, and at the dwell's end she orders
//!    the ADVANCE. The intruder's player then destroys a roster hull: the WITHDRAW comes at
//!    once, inside that ADVANCE's dwell. The organiser reads every one of her orders, and the answers,
//!    in `+sortie/orders`; a player with no part in the roster is refused.
//! 2. **A plan's work point is advanced to** ([`work_point`], at 256 and at 512): the organiser
//!    names it with `+sortie/workpoint`, the plan shows it, and the commander's ADVANCE is to
//!    that cell. (A plan that names none advances to the default: MW.14a's acceptance, whose
//!    plan is drafted in the format every plan had before this unit.)
//! 3. **A player leader's `CONCENTRATE`**
//!    ([`mw14c_a_leaders_concentrate_puts_a_hull_that_holds_the_contact_on_it`]). A captain
//!    leads with Rook Adeyemi in an armed rig on HOLD. The captain marks the intruder's party
//!    Hostile and orders `ALL CONCENTRATE` on it with the roster's comms jammed: the order does
//!    not arrive and the hull holds, firing nothing. The jam lifts: the hull is on the contact
//!    and fires on it. The intruder is destroyed: the hull is on its HOLD again, by itself.
//! 4. **A hull that does not hold the contact keeps its order**
//!    ([`mw14c_a_hull_that_does_not_hold_the_contact_keeps_its_order`]). The same roster with
//!    the hired rig out of sight of the intruder and out of the leader's share range: it
//!    answers NEGATIVE, the kernel is offered nothing, and its HOLD goes on standing.
//! 5. **An NPC-led roster concentrates on the party that fired**
//!    ([`mw14c_an_npc_led_roster_concentrates_on_the_party_that_fired`]). Two hired crew in
//!    armed rigs, no player in a unit. The intruder fires on the roster's carrier; the escort,
//!    nearer, never fires on the roster. The commander holds and names the intruder for both
//!    hulls in one order; both fire on it, and neither fires on the escort.
//! 6. **An absent player's unit does not fire first**
//!    ([`mw14c_an_absent_players_unit_is_sent_no_concentrate_and_does_not_fire_first`]). A
//!    captain leaves their armed rig on HOLD and quits; the plan's commander leads, is fired on,
//!    and concentrates her own hull on the intruder. The captain's rig is sent no order and
//!    fires nothing at a unit that has not fired on it.
//! 7. **A restart gives the same next order**
//!    ([`mw14c_a_restart_with_the_npc_leading_gives_the_same_next_order`]). The server goes
//!    down between the commander's ADVANCE to her own hull and the one she owes the other; on
//!    the next boot the next order is that ADVANCE to the other hull, and her own is not sent
//!    again.
//! 8. **The same run twice gives the same orders**
//!    ([`mw14c_the_same_run_gives_the_same_orders_twice`]): story 5's script on two fresh
//!    worlds.
//!
//! What the stories stand in for, beyond MW.14a's and AP.8's list (fixture hulls with players
//! seated, staff's jam, rigs stood on their cells by a staff hand):
//! - **a hired NPC sits in an armed rig by a refit** (`field_ap8::arm`): no shipped command
//!   seats an NPC in an armed rig's one console;
//! - **a hull is made fragile by a staff hand** (`field_host_rig::fragile`) so that a kill
//!   takes seconds and no dice decide the story;
//! - **the world's clock is stepped by the story** where a dwell is measured: the arena runs
//!   in real time beside it, so a fight can be waited out with the world's clock stopped
//!   ([`stopped`]) and the dwell then counted second by second ([`tick`]).

use std::time::{Duration, Instant};

use mush_core::ObjId;
use mush_tactics::detachment::{DetachmentRoster, LeaderKind, OrderBook, Response, Slot};
use tacsim::arena::ledger::{EventKind, LedgerEvent};
use tacsim::arena::party::Relation;
use tacsim::arena::perception::MindObjective;
use tacsim::arena::state::Classification;
use tacsim::detachment::commander::{DWELL_S, WORK_POINT_CELLS};
use tacsim::detachment::sortie::{FieldObjective, NPC_ORDERS_HEAD};

use crate::driving_rig::{Rig, cid, command, land, on_pad_as_at, run, stocked_tuned, warm};
use crate::field_ap8::{Field, arm, fired, fired_at, hostiles, quit, restart, stand, standing};
use crate::field_host_rig::{
    armed_on, egress, fragile, frame, pump, seat, second_composed, task, until_composed, weapon_of,
};
use crate::field_mw14a::carrier_cell;
use crate::grounds_rig::{AHUNA_FLANK, Ground, Placed, URVARA_REACH};
use crate::roster_rig::{
    Billet, Held, Plan, commit, delivered_to, fence, field_roster, fly_to, operation, orders_of,
    reply,
};
use crate::site_record_rig::staff_member;

/// The shipped crew at Ceres (`content/crew/ceres.json`): the forewoman under a commander's
/// contract, the mining operator, and the escort under a combat contract.
const OKONKWO: &str = "Sana Okonkwo";
const MENDES: &str = "Ilya Mendes";
const ADEYEMI: &str = "Rook Adeyemi";

/// **What a story's field is made of.**
struct Setup {
    /// The ground.
    at: &'static Ground,
    /// The roster.
    plan: Plan,
    /// The plan's work point, named by `+sortie/workpoint`.
    work: Option<(u16, u16)>,
    /// The slots whose rigs are refitted armed.
    armed: Vec<usize>,
    /// Where a staff hand stands a slot's rig; a slot not named stays at its ramp.
    stands: Vec<(usize, Placed)>,
    /// The escort's hull, and the bearing its player fires on to open the field.
    escort: (Placed, &'static str),
    /// The intruder's hull.
    intruder: Placed,
    /// Whether the intruder is fragile (and so has nothing to fire), and a roster slot that
    /// is.
    fragile: (bool, Option<usize>),
    /// Whether the escort is frail (and still armed).
    frail_escort: bool,
    /// Whether to wait for every operator's first frame before the story starts.
    frames: bool,
}

/// MW.14a's ground: the roster left at its ramp, the intruder on the miner's ground and the
/// escort six cells east of it, firing west.
fn at_the_ramp(at: &'static Ground, plan: Plan) -> Setup {
    Setup {
        at,
        plan,
        work: None,
        armed: Vec::new(),
        stands: Vec::new(),
        escort: (((at.read.0 + 6, at.read.1), 270_000), "W"),
        intruder: (at.read, 270_000),
        fragile: (false, None),
        frail_escort: false,
        frames: true,
    }
}

/// AP.8's ground on Ahuna Flank: rig 1 on the miner's ground, rig 2 seven cells east of it, the
/// escort and the intruder east of both, the escort firing east.
fn close_in(plan: Plan) -> Setup {
    let at = &AHUNA_FLANK;
    Setup {
        at,
        plan,
        work: None,
        armed: Vec::new(),
        stands: vec![(1, (at.read, 0)), (2, ((at.read.0 + 7, at.read.1), 90_000))],
        escort: (at.escort, "E"),
        intruder: at.walker,
        fragile: (false, None),
        frail_escort: false,
        frames: true,
    }
}

/// The roster of two hired crew under Okonkwo's command, launched by a captain who stays
/// aboard.
fn npc_plan() -> Plan {
    Plan {
        who: ("Pilot", 1, "Salt Moth"),
        duty: "mining",
        billets: vec![Billet::Hired(OKONKWO), Billet::Hired(MENDES)],
        commander: Some(OKONKWO),
    }
}

/// **Fly a roster onto its ground and open a field over it** with three parties in it.
async fn open(setup: Setup) -> Field {
    let at = setup.at;
    let mut rig = stocked_tuned(|cfg| {
        crate::field_host_rig::tune(cfg);
        // The field holds through the stories' quiet stretches.
        cfg.field.disengage_s = 7_200;
        cfg.field.idle_demote_s = 7_200;
    });
    let flown = fly_to(&mut rig, at, &setup.plan, setup.work, |_, _| {});
    for slot in &setup.armed {
        arm(&mut rig, flown.unit(*slot));
    }
    for (slot, placed) in &setup.stands {
        stand(&mut rig, at.entry, flown.unit(*slot), *placed);
    }
    if let Some(slot) = setup.fragile.1 {
        fragile(&mut rig, flown.unit(slot));
    }
    let (other, _, _) = on_pad_as_at(&mut rig, ("Other", 2, "Tally Moth"), at.site(), 0);
    let (stranger, _, _) = on_pad_as_at(&mut rig, ("Third", 3, "Lone Moth"), at.site(), 0);
    let ((escort_cell, escort_heading), bearing) = setup.escort;
    let escort = armed_on(&mut rig, at.entry, "Red Hound", escort_cell, escort_heading);
    let intruder = armed_on(
        &mut rig,
        at.entry,
        "Grey Hound",
        setup.intruder.0,
        setup.intruder.1,
    );
    if setup.frail_escort {
        frail(&mut rig, escort);
    }
    if setup.fragile.0 {
        fragile(&mut rig, intruder);
    }
    seat(&mut rig, escort, other);
    seat(&mut rig, intruder, stranger);
    warm(&mut rig);
    let mut rx = egress(&mut rig);
    let mut lines = Vec::new();
    for _ in 0..3 {
        let _ = second_composed(&mut rig);
    }
    let (out, queued) = command(&mut rig, other, &format!("+field/fire {bearing}"));
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    land(&mut rig, &queued[0]).expect("the promotion lands");
    assert!(
        until_composed(&mut rig, &mut rx, &mut lines, 15, |rig, _| task(rig)
            .is_some())
        .await,
        "the field's task opens"
    );
    let live = tacsim::field::live_on(rig.tw.world(), &cid(at.entry)).expect("the live record");
    let party = live.party_of(flown.unit(1)).expect("the roster's party");
    assert_eq!(live.party_of(flown.unit(2)), Some(party));
    let others: std::collections::BTreeSet<_> = [escort, intruder]
        .into_iter()
        .filter_map(|unit| live.party_of(unit))
        .collect();
    assert!(
        others.len() == 2 && !others.contains(&party),
        "three parties stand in the field: the roster, the escort and the intruder"
    );
    if setup.frames {
        let crew = flown.crew.clone();
        assert!(
            until_composed(&mut rig, &mut rx, &mut lines, 30, |rig, _| {
                crew.iter().all(|who| frame(rig, *who).is_some())
            })
            .await,
            "the roster's frames arrive"
        );
    }
    Field {
        rig,
        flown,
        other,
        stranger,
        escort,
        intruder,
        rx,
        lines,
    }
}

/// **Make `unit` frail and leave it armed**: `field_host_rig::fragile` on every section but the
/// turret. A hull made fragile all over is admitted to a field with nothing to fire, and the
/// story's intruder has to fire before it is destroyed.
fn frail(rig: &mut Rig, unit: ObjId) {
    crate::driving_rig::staff_hand(rig, |ctx| {
        let cells = mush_tactics::module::unit_of(ctx.w, unit)
            .expect("the unit")
            .clone();
        let ops: Vec<mush_tactics::UnitMutationOp> = cells
            .cells()
            .filter_map(|(addr, cell)| {
                let (section, name) = match addr {
                    mush_tactics::CellAddr::Facet { section, cell, .. }
                    | mush_tactics::CellAddr::Section { section, cell } => {
                        (section.as_str(), cell.as_str())
                    }
                    _ => return None,
                };
                if section == "turret" {
                    return None;
                }
                let value = match name {
                    tacsim::definitions::unit::ARMOR_CELL => cell.min,
                    tacsim::definitions::unit::INTERNALS_CELL => {
                        cell.min.saturating_add(1).min(cell.max)
                    }
                    _ => return None,
                };
                Some(mush_tactics::UnitMutationOp::SetCell {
                    addr: addr.clone(),
                    value,
                })
            })
            .collect();
        let plan = mush_tactics::UnitMutationPlan::new(
            ops,
            None,
            Some(smol_str::SmolStr::new_static("test:mw14c-frail")),
        )
        .expect("a plan");
        mush_tactics::module::apply_plan(ctx, unit, &plan).expect("applied");
    });
}

/// **One world second, and the arena's egress after it.** The dwell is counted in these.
async fn tick(field: &mut Field) {
    let _ = second_composed(&mut field.rig);
    tokio::time::sleep(Duration::from_millis(80)).await;
    let _ = pump(&mut field.rig, &mut field.rx, &mut field.lines);
}

/// [`tick`] until `done` holds, at most `most` world seconds; whether it held.
async fn ticks(field: &mut Field, most: u32, mut done: impl FnMut(&mut Rig) -> bool) -> bool {
    for _ in 0..most {
        if done(&mut field.rig) {
            return true;
        }
        tick(field).await;
    }
    done(&mut field.rig)
}

/// **The arena runs and the world's clock stands still**: the egress is pumped until `done`
/// holds or `secs` of real time pass, and `each` is run once a real second (a player's `LOCK`
/// and `FIRE`). No world second passes, so nothing a dwell is measured in moves.
async fn stopped(
    field: &mut Field,
    secs: u64,
    mut each: impl FnMut(&mut Field),
    mut done: impl FnMut(&Rig) -> bool,
) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    let mut beat = 0_u32;
    while Instant::now() < deadline {
        if beat.is_multiple_of(40) {
            each(field);
        }
        beat += 1;
        tokio::time::sleep(Duration::from_millis(25)).await;
        pump(&mut field.rig, &mut field.rx, &mut field.lines);
        if done(&field.rig) {
            return true;
        }
    }
    false
}

/// **`who`, at `hull`'s controls, keeps a lock on the contact within `within` cells of `cell`
/// and fires**: one pass of the two lines a player types. Several carriers settle on one pad
/// and the field stands each on a cell of its own, so a carrier is named by its exact cell
/// (`within` nought).
fn shoot(field: &mut Field, (who, hull): (ObjId, ObjId), cell: (u16, u16), within: u16) {
    let gun = weapon_of(&field.rig, hull);
    // The contact nearest the cell: two rigs at one work point stand a cell apart.
    let near = frame(&field.rig, who).and_then(|frame| {
        frame
            .contacts
            .iter()
            .filter_map(|contact| {
                let (x, y) = contact.cell?;
                let off = x.abs_diff(cell.0).max(y.abs_diff(cell.1));
                (off <= within).then_some((off, contact))
            })
            .min_by_key(|(off, contact)| (*off, contact.id))
            .map(|(_, contact)| (contact.id, contact.designator.to_string(), frame.lock))
    });
    if let Some((id, designator, lock)) = near
        && lock != Some(id)
    {
        let _ = run(&mut field.rig.tw, who, &format!("LOCK {designator}"));
    }
    let _ = run(&mut field.rig.tw, who, &format!("FIRE {gun}"));
}

/// The world's wall second.
fn now(field: &Field) -> i64 {
    field.rig.tw.world().now()
}

/// The roster's orders a story reads the commander by: `(sequence, slot, words)` of every order
/// `issuer` gave that the book still holds, in sequence order.
fn given_by(field: &mut Field, issuer: ObjId) -> Vec<(u64, u16, String)> {
    let operation = field.flown.operation;
    let mut rows: Vec<(u64, u16, String)> = orders_of(&mut field.rig, operation)
        .into_iter()
        .filter(|held| held.issuer == issuer)
        .map(|held| (held.sequence, held.slot, words(&held)))
        .collect();
    rows.sort();
    rows
}

/// An order's words, as a leader types them.
fn words(held: &Held) -> String {
    match &held.objective {
        None => "RELEASE".to_owned(),
        Some(FieldObjective::Hold) => "HOLD".to_owned(),
        Some(FieldObjective::Withdraw) => "WITHDRAW".to_owned(),
        Some(FieldObjective::Advance { to }) => format!("ADVANCE {},{}", to.0, to.1),
        Some(FieldObjective::Concentrate { designator, .. }) => {
            format!("CONCENTRATE {designator}")
        }
        Some(other) => format!("{other:?}"),
    }
}

/// The latest order `issuer` gave that the book still holds, by its words.
fn latest(field: &mut Field, issuer: ObjId) -> Option<String> {
    given_by(field, issuer).pop().map(|(_, _, words)| words)
}

/// The cell `who`'s own frame puts its unit on.
fn cell_of(rig: &Rig, who: ObjId) -> (u16, u16) {
    frame(rig, who)
        .and_then(|frame| frame.cell)
        .expect("the unit's own cell")
}

/// The site cell `unit` stands on, by its surface placement.
fn placed(rig: &Rig, unit: ObjId) -> (i64, i64) {
    mush_sim::pose_of(rig.tw.world(), unit)
        .map(|(_, pose)| tacsim::surface::excursion::site_cell(pose.x_cm, pose.y_cm))
        .expect("the unit is placed")
}

/// **The designator `who`'s own frame gives the contact standing on `cell`** once the frame
/// has identified it (the contact is classified), waited for.
async fn identified(field: &mut Field, who: ObjId, cell: (u16, u16)) -> String {
    let find = move |rig: &Rig| {
        frame(rig, who).and_then(|frame| {
            frame
                .contacts
                .iter()
                .find(|contact| {
                    matches!(contact.class, Classification::Classified { .. })
                        && contact.cell.is_some_and(|(x, y)| {
                            x.abs_diff(cell.0) <= 1 && y.abs_diff(cell.1) <= 1
                        })
                })
                .map(|contact| contact.designator.to_string())
        })
    };
    assert!(
        field.until(30, |rig, _| find(rig).is_some()).await,
        "#{}'s frame identifies the contact at {cell:?}: {:?}",
        who.0,
        frame(&field.rig, who).map(|frame| frame
            .contacts
            .iter()
            .map(|contact| (contact.designator.to_string(), contact.cell, contact.class))
            .collect::<Vec<_>>())
    );
    find(&field.rig).expect("identified")
}

/// The places the roster's crew stand in, for staff's jam.
fn places(field: &Field) -> Vec<ObjId> {
    let world = field.rig.tw.world();
    let spatial = world
        .module_state::<mush_sim::SpatialState>()
        .expect("the spatial state");
    let places: std::collections::BTreeSet<ObjId> = field
        .flown
        .crew
        .iter()
        .filter_map(|who| mush_propagation::context::place_of(spatial, world, *who))
        .collect();
    assert!(!places.is_empty(), "the crew are somewhere");
    places.into_iter().collect()
}

/// Staff declare interference of `level` where the roster's crew stand.
fn jam(field: &mut Field, staff: ObjId, level: u8) {
    for place in places(field) {
        let said = run(
            &mut field.rig.tw,
            staff,
            &format!("@prop/jam #{}={level}", place.0),
        );
        assert!(level == 0 || said.contains("Interference at"), "{said}");
    }
}

/// What `+sortie/orders` prints for `who` over the roster's operation.
fn orders_report(field: &mut Field, who: ObjId) -> String {
    let line = format!(
        "+sortie/orders {}:{}",
        field.flown.operation.organizer.0,
        field.flown.selector()
    );
    run(&mut field.rig.tw, who, &line)
}

/// The record `slot`'s delivered `CONCENTRATE` left of what the hull goes back to: the earlier
/// order's words, or `None` when there is no such record.
fn goes_back_to(field: &mut Field, slot: u16) -> Option<Option<FieldObjective>> {
    let record = operation(&mut field.rig, field.flown.operation)?;
    let orders = record.orders?;
    let slot = Slot::new(slot).expect("a slot");
    let kept = orders.concentrating.get(&slot)?;
    Some(
        kept.before
            .as_ref()
            .and_then(|order| match &order.directive {
                mush_tactics::detachment::Directive::Objective(payload) => {
                    Some(payload.objective.clone())
                }
                mush_tactics::detachment::Directive::Cancel => None,
            }),
    )
}

/// The IC millisecond the roster's book stamps on the order `sequence` gave `slot`.
fn issued_ic_ms(field: &mut Field, slot: u16, sequence: u64) -> Option<u64> {
    let record = operation(&mut field.rig, field.flown.operation)?;
    let roster = DetachmentRoster::restore(record.command.as_deref()?).ok()?;
    let book = OrderBook::restore(&record.orders.as_deref()?.book, &roster).ok()?;
    let slot = Slot::new(slot).ok()?;
    book.pending(slot)
        .into_iter()
        .chain(book.delivered(slot).map(|delivered| &delivered.order))
        .find(|order| order.sequence == sequence)
        .map(|order| order.issued_at_ms)
}

/// **Story 1** (this module's head): the dwell, the loss, and the organiser's read.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // one story, in order
async fn mw14c_a_posture_stands_for_the_dwell_and_a_loss_withdraws_at_once() {
    let at = &AHUNA_FLANK;
    let mut setup = at_the_ramp(at, npc_plan());
    // The escort is the party that comes into the commander's sight and leaves it: it fires
    // first, so it is frail and still armed. The roster's second hull is fragile. The intruder
    // is whole: it ends the escort, and later the hull.
    setup.fragile = (false, Some(2));
    setup.frail_escort = true;
    let mut field = open(setup).await;
    let (okonkwo, mendes) = (field.flown.operator(1), field.flown.operator(2));
    let (organiser, stranger) = (field.flown.organiser, field.stranger);
    let (other, escort, intruder) = (field.other, field.escort, field.intruder);
    let operation = field.flown.operation;
    let dwell = i64::from(DWELL_S);
    assert_eq!(
        field.leader(),
        Some((okonkwo, LeaderKind::Npc)),
        "the plan's NPC commander leads"
    );

    // (1) Nothing hostile in her sight: she orders the advance, and both hulls take it.
    assert!(
        field
            .until(90, |rig, _| {
                [okonkwo, mendes]
                    .into_iter()
                    .all(|who| matches!(standing(rig, who), Some(MindObjective::Advance { .. })))
            })
            .await,
        "the commander orders the advance: {:?}",
        orders_of(&mut field.rig, operation)
    );
    // Both rigs reach the work point, in the open where the story's two other hulls hold them.
    let Some(MindObjective::Advance { to: work }) = standing(&field.rig, mendes) else {
        unreachable!("the advance stands")
    };
    let arrived = field
        .until(120, |rig, _| {
            [okonkwo, mendes].into_iter().all(|who| {
                frame(rig, who)
                    .and_then(|frame| frame.cell)
                    .is_some_and(|(x, y)| x.abs_diff(work.0) <= 2 && y.abs_diff(work.1) <= 2)
            })
        })
        .await;
    assert!(
        arrived,
        "both rigs reach the work point {work:?}: {:?} / {:?}",
        frame(&field.rig, okonkwo).and_then(|frame| frame.cell),
        frame(&field.rig, mendes).and_then(|frame| frame.cell)
    );
    // The advance grows older than the dwell: the next change of posture is not held by it.
    field.pass(dwell + 5);
    field.wait(1).await;
    assert!(
        latest(&mut field, okonkwo).is_some_and(|words| words.starts_with("ADVANCE")),
        "nothing but the advance was ordered: {:?}",
        given_by(&mut field, okonkwo)
    );

    // (2) A Hostile comes into her sight: the intruder fires on the roster's carrier, with
    // the world's clock stopped. She orders HOLD at once.
    let pad = carrier_cell(&field.rig, okonkwo, &[okonkwo, mendes])
        .expect("her frame holds her own carrier");
    let seen = stopped(
        &mut field,
        90,
        |field| shoot(field, (other, escort), pad, 0),
        |rig| hostiles(rig, okonkwo) > 0,
    )
    .await;
    let gun = weapon_of(&field.rig, escort);
    let tried = run(&mut field.rig.tw, other, &format!("FIRE {gun}"));
    assert!(
        seen,
        "the intruder's fire on the carrier at {pad:?} puts a Hostile in her sight: FIRE says          {tried}; its frame {:?}; rounds {}",
        frame(&field.rig, other).map(|frame| (
            frame.cell,
            frame.lock,
            frame.wreck,
            frame
                .contacts
                .iter()
                .map(|contact| (
                    contact.designator.to_string(),
                    contact.cell,
                    contact.relation
                ))
                .collect::<Vec<_>>()
        )),
        fired(&field.lines, escort)
    );
    field.cease((other, escort));
    let ordered = ticks(&mut field, 6, |rig| {
        orders_of(rig, operation)
            .iter()
            .any(|held| held.issuer == okonkwo && held.objective == Some(FieldObjective::Hold))
    })
    .await;
    assert!(
        ordered,
        "with the advance older than the dwell the HOLD is ordered at once: {:?}",
        given_by(&mut field, okonkwo)
    );
    let held_at = now(&field);
    let hold_sequence = given_by(&mut field, okonkwo)
        .into_iter()
        .filter(|(_, _, words)| words == "HOLD")
        .map(|(sequence, _, _)| sequence)
        .min()
        .expect("the HOLD");
    let both = ticks(&mut field, 14, |rig| {
        [okonkwo, mendes]
            .into_iter()
            .all(|who| standing(rig, who) == Some(MindObjective::Hold))
    })
    .await;
    assert!(
        both && now(&field) < held_at + dwell - 4,
        "both hulls hold well inside the dwell: {:?} / {:?}, {} s after the HOLD",
        standing(&field.rig, okonkwo),
        standing(&field.rig, mendes),
        now(&field) - held_at
    );

    // (3) The Hostile leaves her sight: the escort's player destroys the intruder, with the
    // world's clock stopped, and her frame drops the contact.
    let target = cell_of(&field.rig, other);
    let gone = stopped(
        &mut field,
        150,
        |field| shoot(field, (stranger, intruder), target, 1),
        |rig| hostiles(rig, okonkwo) == 0,
    )
    .await;
    assert!(
        gone,
        "the escort is destroyed and out of her sight: wreck {:?}, her contacts {:?}",
        frame(&field.rig, other).map(|frame| frame.wreck),
        frame(&field.rig, okonkwo).map(|frame| frame
            .contacts
            .iter()
            .map(|contact| (contact.designator.to_string(), contact.relation))
            .collect::<Vec<_>>())
    );
    field.cease((stranger, intruder));
    // Inside the dwell nothing changes: no order is given, and both hulls still hold.
    while now(&field) < held_at + dwell - 1 {
        tick(&mut field).await;
        assert_eq!(
            latest(&mut field, okonkwo).as_deref(),
            Some("HOLD"),
            "{} s after the HOLD, inside the dwell, the posture is not changed: {:?}",
            now(&field) - held_at,
            given_by(&mut field, okonkwo)
        );
    }
    for who in [okonkwo, mendes] {
        assert_eq!(standing(&field.rig, who), Some(MindObjective::Hold));
    }
    // The dwell is out: the advance is ordered again.
    let advanced = ticks(&mut field, 8, |rig| {
        orders_of(rig, operation).iter().any(|held| {
            held.issuer == okonkwo
                && held.sequence > hold_sequence
                && matches!(held.objective, Some(FieldObjective::Advance { .. }))
        })
    })
    .await;
    assert!(
        advanced && now(&field) >= held_at + dwell,
        "once the dwell is out she orders the advance: {:?}, {} s after the HOLD",
        given_by(&mut field, okonkwo),
        now(&field) - held_at
    );
    let advanced_at = now(&field);

    // (4) A loss, a few seconds into that advance's dwell: the escort's player destroys the
    // other hull, with the world's clock stopped. The WITHDRAW is not delayed.
    let lost = stopped(
        &mut field,
        150,
        |field| {
            // The rig is driving for the work point: it is named by where it is now.
            let victim = cell_of(&field.rig, mendes);
            shoot(field, (stranger, intruder), victim, 1);
        },
        |rig| frame(rig, mendes).is_some_and(|frame| frame.wreck),
    )
    .await;
    let gun = weapon_of(&field.rig, intruder);
    let tried = run(&mut field.rig.tw, stranger, &format!("FIRE {gun}"));
    let status = run(&mut field.rig.tw, stranger, "WEAPONSTATUS");
    assert!(
        lost,
        "the roster's second hull is destroyed: FIRE says {tried}; {status}; the intruder's frame {:?}; the rig at {:?}; rounds at it {}, at the escort {}",
        frame(&field.rig, stranger).map(|frame| (
            frame.cell,
            frame.lock,
            frame
                .contacts
                .iter()
                .map(|contact| (contact.id, contact.designator.to_string(), contact.cell))
                .collect::<Vec<_>>()
        )),
        frame(&field.rig, mendes).map(|frame| (frame.cell, frame.wreck, frame.immobile)),
        fired_at(&field.lines, intruder, field.flown.unit(2)),
        fired_at(&field.lines, intruder, escort)
    );
    field.cease((stranger, intruder));
    let mut refused = Vec::new();
    let mut home = false;
    for _ in 0..12 {
        home = orders_of(&mut field.rig, operation)
            .iter()
            .any(|held| held.issuer == okonkwo && held.objective == Some(FieldObjective::Withdraw));
        if home {
            break;
        }
        refused.extend(second_composed(&mut field.rig));
        tokio::time::sleep(Duration::from_millis(80)).await;
        let _ = pump(&mut field.rig, &mut field.rx, &mut field.lines);
    }
    assert!(
        home && now(&field) < advanced_at + dwell,
        "a loss withdraws at once, {} s into the advance's dwell of {dwell}: {:?}; leader          {:?}; refused {refused:?}; kept {:?}",
        now(&field) - advanced_at,
        given_by(&mut field, okonkwo),
        field.leader(),
        crate::roster_rig::operation(&mut field.rig, operation)
            .and_then(|record| record.orders)
            .map(|orders| orders
                .given
                .iter()
                .map(|given| (given.sequence, given.slot.get(), given.at_ms))
                .collect::<Vec<_>>())
    );

    // (5) The organiser reads what their commander ordered and how it was answered.
    field.wait(3).await;
    let report = orders_report(&mut field, organiser);
    assert!(report.contains(NPC_ORDERS_HEAD), "{report}");
    let hers: Vec<&str> = report
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with(&format!("{OKONKWO} to slot ")))
        .collect();
    let said = |words: &str| hers.iter().position(|line| line.contains(words));
    let (first, hold, withdraw) = (said(": ADVANCE "), said(": HOLD;"), said(": WITHDRAW;"));
    assert!(
        matches!((first, hold, withdraw), (Some(a), Some(h), Some(w)) if a < h && h < w),
        "her advance, her hold and her withdrawal, in order: {hers:?}"
    );
    assert!(
        hers.iter().any(|line| line.ends_with("WILCO received."))
            && hers.iter().all(|line| line.chars().count() <= 120),
        "each with its answer once that reached her, inside 120 columns: {hers:?}"
    );
    // A player with no part in the roster reads none of it.
    let refused = orders_report(&mut field, stranger);
    assert!(
        !refused.contains(OKONKWO)
            && !refused.contains(NPC_ORDERS_HEAD)
            && !refused.contains("DT:"),
        "a stranger is refused the roster's orders: {refused}"
    );
}

/// **Story 2 on `at`'s ground** (this module's head): the plan names `work`, and the
/// commander's ADVANCE is to that cell.
async fn work_point(at: &'static Ground, work: (u16, u16), drive: bool) {
    let mut setup = at_the_ramp(at, npc_plan());
    setup.work = Some(work);
    let mut field = open(setup).await;
    let (okonkwo, mendes) = (field.flown.operator(1), field.flown.operator(2));
    let operation = field.flown.operation;
    let ramp = placed(&field.rig, field.flown.unit(2));
    let off = (i64::from(work.0) - ramp.0)
        .abs()
        .max((i64::from(work.1) - ramp.1).abs());
    assert!(
        off > i64::from(WORK_POINT_CELLS) + 1,
        "the named point is not the default: {work:?} is {off} cells off the ramp at {ramp:?}"
    );
    let ordered = field
        .until(90, |rig, _| {
            [okonkwo, mendes]
                .into_iter()
                .all(|who| standing(rig, who) == Some(MindObjective::Advance { to: work }))
        })
        .await;
    assert!(
        ordered,
        "the commander advances the roster to the plan's work point {work:?}: {:?} / {:?}; {:?}",
        standing(&field.rig, okonkwo),
        standing(&field.rig, mendes),
        orders_of(&mut field.rig, operation)
    );
    for slot in [1, 2] {
        let held = delivered_to(&mut field.rig, operation, slot).expect("a delivered order");
        assert_eq!(held.issuer, okonkwo);
        assert_eq!(held.objective, Some(FieldObjective::Advance { to: work }));
    }
    if drive {
        let range = |rig: &Rig| {
            frame(rig, mendes).map(|frame| {
                let (dx, dy) = (
                    frame.pose.x_cm - (i64::from(work.0) * 1_000 + 500),
                    frame.pose.y_cm - (i64::from(work.1) * 1_000 + 500),
                );
                i64::try_from((dx * dx + dy * dy).unsigned_abs().isqrt()).unwrap_or(i64::MAX)
            })
        };
        let from = range(&field.rig).expect("the range to the work point");
        let closing = field
            .until(90, |rig, _| {
                range(rig).is_some_and(|cm| cm + 1_500 <= from || cm <= 2_000)
            })
            .await;
        assert!(
            closing,
            "the rig drives for the plan's work point: {:?} from {from} cm",
            range(&field.rig)
        );
    }
}

/// **Story 2 at 256** on Ahuna Flank ([`work_point`]): the miner's unread pocket.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mw14c_a_plans_work_point_is_advanced_to() {
    let at = &AHUNA_FLANK;
    let work = (
        u16::try_from(at.core.0).expect("a cell"),
        u16::try_from(at.core.1).expect("a cell"),
    );
    work_point(at, work, true).await;
}

/// **Story 2 at 512** on Urvara Reach ([`work_point`]): a cell past 255 on both axes, which no
/// 256 site holds.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mw14c_a_plans_work_point_is_advanced_to_at_512() {
    work_point(&URVARA_REACH, (262, 268), false).await;
}

/// A combat roster: the captain in rig 1, Rook Adeyemi in an armed rig 2.
fn escorted() -> Plan {
    Plan {
        who: ("Pilot", 1, "Salt Moth"),
        duty: "combat",
        billets: vec![Billet::Leader, Billet::Hired(ADEYEMI)],
        commander: None,
    }
}

/// The captain marks the intruder's party Hostile and identifies the intruder in their own
/// cockpit; its designator there.
async fn marked(field: &mut Field) -> String {
    let captain = field.flown.organiser;
    let letter = tacsim::field::live_on(field.rig.tw.world(), &cid(AHUNA_FLANK.entry))
        .and_then(|live| live.party_of(field.intruder))
        .expect("the intruder's party")
        .letter();
    let said = run(
        &mut field.rig.tw,
        captain,
        &format!("+field/mark {letter}=hostile"),
    );
    assert!(
        field.until(30, |rig, _| hostiles(rig, captain) > 0).await,
        "the captain's own frame holds the intruder Hostile: {said}"
    );
    let cell = cell_of(&field.rig, field.stranger);
    identified(field, captain, cell).await
}

/// The captain's `ALL CONCENTRATE <designator>`, under the fence `+sortie/orders` prints.
fn concentrate_all(field: &mut Field, designator: &str) -> String {
    let captain = field.flown.organiser;
    let fenced = fence(&mut field.rig, captain, &field.flown);
    commit(
        &mut field.rig,
        captain,
        &format!("ORDER {fenced} ALL CONCENTRATE {designator}"),
    )
}

/// The `CONCENTRATE`s the kernel was given for `unit`, off the field's ledger.
fn engagements_commanded(lines: &[LedgerEvent], unit: ObjId) -> usize {
    lines
        .iter()
        .filter(|line| {
            matches!(&line.kind, EventKind::DetachmentCommanded { unit: to, command }
            if *to == unit && command.directive.is_some_and(|directive| matches!(
                directive.objective,
                tacsim::deliberation::order::Objective::Engage { .. }
            )))
        })
        .count()
}

/// **Story 3** (this module's head): a player leader's `CONCENTRATE`, jammed, taken and ended.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // one story, in order
async fn mw14c_a_leaders_concentrate_puts_a_hull_that_holds_the_contact_on_it() {
    let mut setup = close_in(escorted());
    setup.armed = vec![2];
    setup.fragile = (true, None);
    let mut field = open(setup).await;
    let (captain, adeyemi) = (field.flown.organiser, field.flown.operator(2));
    let (lead_rig, tank) = (field.flown.unit(1), field.flown.unit(2));
    let (intruder, escort) = (field.intruder, field.escort);
    let operation = field.flown.operation;
    assert_eq!(field.leader(), Some((captain, LeaderKind::Player)));

    // The hull stands on its leader's HOLD.
    field.order("HOLD");
    assert!(
        field
            .until(60, |rig, _| standing(rig, adeyemi)
                == Some(MindObjective::Hold))
            .await,
        "the HOLD stands: {:?}",
        orders_of(&mut field.rig, operation)
    );
    let hold = delivered_to(&mut field.rig, operation, 2).expect("the HOLD");
    let designator = marked(&mut field).await;
    assert!(
        field.until(30, |rig, _| hostiles(rig, adeyemi) > 0).await,
        "the hired hull holds the intruder Hostile itself"
    );

    // A contact under a provisional number, or none at all, is refused by name.
    let fenced = fence(&mut field.rig, captain, &field.flown);
    let nothing = run(
        &mut field.rig.tw,
        captain,
        &format!("ORDER {fenced} ALL CONCENTRATE ZZ9"),
    );
    assert!(
        nothing.contains(tacsim::detachment::sortie::CONCENTRATE_NO_CONTACT),
        "{nothing}"
    );

    // Jammed: the order is admitted and sent and does not arrive. The hull holds and fires
    // nothing, though a Hostile stands in front of it.
    let staff = staff_member(&mut field.rig);
    jam(&mut field, staff, 4);
    let sent = concentrate_all(&mut field, &designator);
    let in_flight = field
        .until(30, |rig, _| {
            orders_of(rig, operation).iter().any(|held| {
                held.slot == 2
                    && !held.delivered
                    && matches!(held.objective, Some(FieldObjective::Concentrate { .. }))
            })
        })
        .await;
    assert!(
        in_flight,
        "the CONCENTRATE is in flight to the hired hull: {sent}; {:?}",
        orders_of(&mut field.rig, operation)
    );
    field.wait(3).await;
    let still = delivered_to(&mut field.rig, operation, 2).expect("the HOLD");
    assert_eq!(
        (still.sequence, standing(&field.rig, adeyemi)),
        (hold.sequence, Some(MindObjective::Hold)),
        "a jammed CONCENTRATE does not arrive"
    );
    assert_eq!(fired(&field.lines, tank), 0, "a hull on HOLD fires nothing");

    // The jam lifts: the order arrives, the hull is on the contact and fires on it.
    jam(&mut field, staff, 0);
    // (The intruder is fragile, so the engagement may be over a frame or two after the first
    // round: the engagement and the fire are each watched for.)
    let mut engaged = false;
    let on_it = field
        .until(90, |rig, lines| {
            engaged |= matches!(
                standing(rig, adeyemi),
                Some(MindObjective::Engage { target: Some(_) })
            );
            engaged && fired_at(lines, tank, intruder) > 0
        })
        .await;
    let taken = delivered_to(&mut field.rig, operation, 2).expect("the CONCENTRATE");
    assert!(
        on_it,
        "the hull concentrates on the contact its leader named: engaged {engaged}, now {:?}, \
         {} rounds; {:?}",
        standing(&field.rig, adeyemi),
        fired_at(&field.lines, tank, intruder),
        orders_of(&mut field.rig, operation)
    );
    assert_eq!(
        engagements_commanded(&field.lines, tank),
        1,
        "the kernel was given the one engagement"
    );
    assert!(
        matches!(&taken.objective, Some(FieldObjective::Concentrate { designator: named, .. })
            if named.as_str() == designator),
        "{taken:?}"
    );
    assert_eq!((taken.issuer, taken.issuer_unit), (captain, lead_rig));
    assert_eq!(
        goes_back_to(&mut field, 2),
        Some(Some(FieldObjective::Hold)),
        "the roster's history records the HOLD it goes back to"
    );

    // The contact is destroyed: the hull is on its HOLD again, by itself, with no new order.
    let stranger = field.stranger;
    let back = field
        .until(150, |rig, _| {
            frame(rig, stranger).is_some_and(|frame| frame.wreck)
                && standing(rig, adeyemi) == Some(MindObjective::Hold)
        })
        .await;
    assert!(
        back,
        "once the contact is destroyed the hull stands on its earlier order: {:?}, intruder \
         wreck {:?}",
        standing(&field.rig, adeyemi),
        frame(&field.rig, field.stranger).map(|frame| frame.wreck)
    );
    let last = delivered_to(&mut field.rig, operation, 2).expect("the CONCENTRATE");
    assert_eq!(
        last.sequence, taken.sequence,
        "no order was sent to bring it back: the return is the hull's own"
    );
    assert_eq!(
        reply(&mut field.rig, captain, &field.flown, 2).as_deref(),
        Some("WILCO received")
    );
    // It fired on the contact it was given and on nothing else.
    assert_eq!(fired_at(&field.lines, tank, escort), 0);
    assert_eq!(
        fired(&field.lines, tank),
        fired_at(&field.lines, tank, intruder)
    );
    let rounds = fired(&field.lines, tank);
    field.wait(4).await;
    assert_eq!(
        fired(&field.lines, tank),
        rounds,
        "back on HOLD it fires nothing"
    );
}

/// **Story 4** (this module's head): a hull that does not hold the contact keeps its order.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mw14c_a_hull_that_does_not_hold_the_contact_keeps_its_order() {
    let at = &AHUNA_FLANK;
    let mut setup = close_in(escorted());
    setup.armed = vec![2];
    // The hired rig stands 67 cells west of the intruder, beyond its own sight (60 cells), and
    // 54 cells from its leader's rig, beyond the range a party shares its contacts over (30).
    setup.stands = vec![
        (1, (at.read, 0)),
        (2, ((at.walker.0.0 - 67, at.walker.0.1), 90_000)),
    ];
    let mut field = open(setup).await;
    let (captain, adeyemi) = (field.flown.organiser, field.flown.operator(2));
    let tank = field.flown.unit(2);
    let operation = field.flown.operation;

    field.order("HOLD");
    assert!(
        field
            .until(60, |rig, _| standing(rig, adeyemi)
                == Some(MindObjective::Hold))
            .await,
        "the HOLD stands: {:?}",
        orders_of(&mut field.rig, operation)
    );
    let hold = delivered_to(&mut field.rig, operation, 2).expect("the HOLD");
    let row = frame(&field.rig, adeyemi)
        .and_then(|frame| frame.order)
        .expect("the HOLD's row");
    let designator = marked(&mut field).await;
    field.wait(4).await;
    let holds = |rig: &Rig| {
        frame(rig, adeyemi).is_some_and(|frame| {
            frame
                .contacts
                .iter()
                .any(|contact| contact.designator.to_string() == designator)
        })
    };
    assert!(
        !holds(&field.rig),
        "the hired hull does not hold the contact its leader named: {:?}",
        frame(&field.rig, adeyemi).map(|frame| frame
            .contacts
            .iter()
            .map(|contact| (contact.designator.to_string(), contact.cell))
            .collect::<Vec<_>>())
    );

    let sent = concentrate_all(&mut field, &designator);
    let answered = field
        .until(90, |rig, _| {
            orders_of(rig, operation).iter().any(|held| {
                held.slot == 2
                    && held.delivered
                    && matches!(held.objective, Some(FieldObjective::Concentrate { .. }))
            })
        })
        .await;
    assert!(
        answered,
        "the CONCENTRATE is delivered: {sent}; {:?}",
        orders_of(&mut field.rig, operation)
    );
    let negative = field
        .until(60, |rig, _| {
            operation_answer(rig, operation, 2) == Some(Response::Negative)
        })
        .await;
    assert!(negative, "it answers NEGATIVE");
    field.wait(4).await;
    assert!(!holds(&field.rig), "it still does not hold the contact");
    // It keeps the order it had: the same HOLD on the same row, nothing offered to the kernel
    // for it, and no round fired.
    assert_eq!(
        frame(&field.rig, adeyemi).and_then(|frame| frame.order),
        Some(row),
        "its HOLD goes on standing, untouched"
    );
    assert_eq!(
        engagements_commanded(&field.lines, tank),
        0,
        "the kernel was offered no engagement for it"
    );
    assert_eq!(fired(&field.lines, tank), 0);
    assert_eq!(
        goes_back_to(&mut field, 2),
        Some(Some(FieldObjective::Hold)),
        "the history keeps what it stands on: order {}",
        hold.sequence
    );
    let mut read = None;
    for _ in 0..30 {
        read = reply(&mut field.rig, captain, &field.flown, 2);
        if read.as_deref() == Some("NEGATIVE received") {
            break;
        }
        field.wait(1).await;
    }
    assert_eq!(
        read.as_deref(),
        Some("NEGATIVE received"),
        "its leader reads the NEGATIVE once it arrives"
    );
}

/// The answer the roster's book records for `slot`'s delivered order.
fn operation_answer(
    rig: &mut Rig,
    id: tacsim::detachment::sortie::OperationId,
    slot: u16,
) -> Option<Response> {
    let record = operation(rig, id)?;
    let roster = DetachmentRoster::restore(record.command.as_deref()?).ok()?;
    let book = OrderBook::restore(&record.orders.as_deref()?.book, &roster).ok()?;
    book.delivered(Slot::new(slot).ok()?)?
        .acknowledgement
        .map(|answer| answer.response)
}

/// **The field of stories 5 and 8**: two hired crew in armed rigs under Okonkwo's command, the
/// plan's work point on her own cell so the advance moves them little, the escort nearer the
/// roster than the intruder.
async fn armed_roster() -> Field {
    let at = &AHUNA_FLANK;
    let mut setup = close_in(npc_plan());
    setup.armed = vec![1, 2];
    setup.work = Some((
        u16::try_from(at.read.0).expect("a cell"),
        u16::try_from(at.read.1).expect("a cell"),
    ));
    open(setup).await
}

/// **The script to the commander's `CONCENTRATE`**: the advance delivered and aged past the
/// dwell, the intruder's fire on the roster's carrier, the HOLD, and both hulls on the contact
/// she named. The designator she named.
async fn to_the_concentrate(field: &mut Field) -> String {
    let (okonkwo, mendes) = (field.flown.operator(1), field.flown.operator(2));
    let (stranger, intruder) = (field.stranger, field.intruder);
    let operation = field.flown.operation;
    assert_eq!(field.leader(), Some((okonkwo, LeaderKind::Npc)));
    assert!(
        field
            .until(90, |rig, _| {
                [okonkwo, mendes]
                    .into_iter()
                    .all(|who| matches!(standing(rig, who), Some(MindObjective::Advance { .. })))
            })
            .await,
        "the commander orders the advance: {:?}",
        orders_of(&mut field.rig, operation)
    );
    field.pass(i64::from(DWELL_S) + 5);
    field.wait(2).await;
    let pad = carrier_cell(&field.rig, okonkwo, &[okonkwo, mendes])
        .expect("her frame holds her own carrier");
    let seen = stopped(
        field,
        120,
        |field| shoot(field, (stranger, intruder), pad, 0),
        |rig| hostiles(rig, okonkwo) > 0 && hostiles(rig, mendes) > 0,
    )
    .await;
    assert!(
        seen,
        "the intruder's fire on the carrier at {pad:?} makes it Hostile to the roster"
    );
    field.cease((stranger, intruder));
    let on_it = field
        .until(120, |rig, _| {
            [okonkwo, mendes].into_iter().all(|who| {
                matches!(
                    standing(rig, who),
                    Some(MindObjective::Engage { target: Some(_) })
                )
            })
        })
        .await;
    assert!(
        on_it,
        "both hulls are on the contact their commander named: {:?} / {:?}; {:?}",
        standing(&field.rig, okonkwo),
        standing(&field.rig, mendes),
        orders_of(&mut field.rig, operation)
    );
    let named = delivered_to(&mut field.rig, operation, 1).expect("her own order");
    match named.objective {
        Some(FieldObjective::Concentrate { designator, .. }) => designator.to_string(),
        other => panic!("slot 1 holds her CONCENTRATE: {other:?}"),
    }
}

/// **Story 5** (this module's head): an NPC-led roster concentrates on the party that fired.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mw14c_an_npc_led_roster_concentrates_on_the_party_that_fired() {
    let mut field = armed_roster().await;
    let okonkwo = field.flown.operator(1);
    let (lead_rig, next_rig) = (field.flown.unit(1), field.flown.unit(2));
    let (organiser, intruder, escort) = (field.flown.organiser, field.intruder, field.escort);
    let operation = field.flown.operation;
    let designator = to_the_concentrate(&mut field).await;

    // One contact for the whole roster, in one order: hers, from her unit, naming the
    // intruder as her own frame calls it.
    let hers = cell_of(&field.rig, field.stranger);
    assert_eq!(identified(&mut field, okonkwo, hers).await, designator);
    let orders: Vec<Held> = [1, 2]
        .into_iter()
        .map(|slot| delivered_to(&mut field.rig, operation, slot).expect("a delivered order"))
        .collect();
    for held in &orders {
        assert_eq!((held.issuer, held.issuer_unit), (okonkwo, lead_rig));
        assert_eq!(words(held), format!("CONCENTRATE {designator}"));
        assert_eq!(
            goes_back_to(&mut field, held.slot),
            Some(Some(FieldObjective::Hold)),
            "under it the hull stands on her HOLD"
        );
    }
    assert_eq!(
        orders[0].sequence, orders[1].sequence,
        "one order reached both hulls"
    );
    // Both fire on it, and neither fires on the third party, which stands nearer.
    let firing = field
        .until(120, |_, lines| {
            [lead_rig, next_rig]
                .into_iter()
                .all(|rig| fired_at(lines, rig, intruder) > 0)
        })
        .await;
    assert!(
        firing,
        "both hulls fire on the contact: {} / {} rounds",
        fired_at(&field.lines, lead_rig, intruder),
        fired_at(&field.lines, next_rig, intruder)
    );
    field.wait(5).await;
    for rig in [lead_rig, next_rig] {
        assert_eq!(
            fired_at(&field.lines, rig, escort),
            0,
            "the third party is never fired on"
        );
        assert_eq!(
            fired(&field.lines, rig),
            fired_at(&field.lines, rig, intruder),
            "every round is at the one contact"
        );
    }
    let escort_row = frame(&field.rig, okonkwo)
        .and_then(|frame| {
            let near = frame
                .contacts
                .iter()
                .min_by_key(|contact| contact.range_cm)?;
            Some((near.relation, near.designator.to_string()))
        })
        .expect("her nearest contact");
    assert!(
        escort_row.0 != Relation::Hostile && escort_row.1 != designator,
        "the nearest contact in her sight is not Hostile, and is not the one she named: \
         {escort_row:?}"
    );
    // No order of hers names any other contact.
    assert!(
        given_by(&mut field, okonkwo)
            .iter()
            .filter(|(_, _, words)| words.starts_with("CONCENTRATE"))
            .all(|(_, _, words)| *words == format!("CONCENTRATE {designator}")),
        "{:?}",
        given_by(&mut field, okonkwo)
    );
    // The organiser reads the order and its answers.
    let seen = field
        .until(30, |rig, _| {
            let report = run(&mut rig.tw, organiser, "+sortie/orders 1:1");
            report
                .lines()
                .filter(|line| {
                    line.contains(&format!(": CONCENTRATE {designator}; WILCO received."))
                })
                .count()
                == 2
        })
        .await;
    assert!(
        seen,
        "the organiser reads her CONCENTRATE and both answers: {}",
        orders_report(&mut field, organiser)
    );
}

/// **Story 6** (this module's head): an absent player's unit is sent no `CONCENTRATE` and
/// does not fire first.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mw14c_an_absent_players_unit_is_sent_no_concentrate_and_does_not_fire_first() {
    let at = &AHUNA_FLANK;
    let mut setup = close_in(Plan {
        who: ("Pilot", 1, "Salt Moth"),
        duty: "mining",
        billets: vec![Billet::Leader, Billet::Hired(OKONKWO)],
        commander: Some(OKONKWO),
    });
    setup.armed = vec![1, 2];
    setup.work = Some((
        u16::try_from(at.read.0 + 7).expect("a cell"),
        u16::try_from(at.read.1).expect("a cell"),
    ));
    let mut field = open(setup).await;
    let (captain, okonkwo) = (field.flown.organiser, field.flown.operator(2));
    let (own_rig, her_rig) = (field.flown.unit(1), field.flown.unit(2));
    let (stranger, intruder) = (field.stranger, field.intruder);
    let operation = field.flown.operation;
    assert_eq!(field.leader(), Some((captain, LeaderKind::Player)));

    // The captain leaves their rig on HOLD and quits: it passes to its mind at once, and the
    // plan's commander leads.
    let set = run(&mut field.rig.tw, captain, "MIND POSTURE HOLD");
    assert_eq!(
        tacsim::posture::record::of(field.rig.tw.world(), own_rig),
        tacsim::posture::Posture::Hold,
        "{set}"
    );
    quit(&mut field.rig, captain);
    assert!(
        field
            .until(60, |rig, _| {
                tacsim::field::absent::minded(rig.tw.world(), own_rig)
                    && field_roster(rig, operation)
                        .and_then(|roster| roster.leader())
                        .is_some_and(|leader| leader.character == okonkwo)
            })
            .await,
        "the captain's rig is its mind's and the commander leads"
    );
    assert!(
        field
            .until(90, |rig, _| matches!(
                standing(rig, okonkwo),
                Some(MindObjective::Advance { .. })
            ))
            .await,
        "the commander orders her own hull's advance: {:?}",
        orders_of(&mut field.rig, operation)
    );
    field.pass(i64::from(DWELL_S) + 5);
    field.wait(2).await;

    // The intruder fires on the roster's carrier, never on the captain's rig. The commander
    // holds and concentrates her own hull on it.
    let pad = carrier_cell(&field.rig, okonkwo, &[captain, okonkwo])
        .expect("her frame holds her carrier, apart from the roster's two rigs");
    let seen = stopped(
        &mut field,
        120,
        |field| shoot(field, (stranger, intruder), pad, 0),
        |rig| hostiles(rig, okonkwo) > 0,
    )
    .await;
    assert!(seen, "the intruder's fire on the carrier makes it Hostile");
    field.cease((stranger, intruder));
    let on_it = field
        .until(120, |rig, lines| {
            matches!(
                standing(rig, okonkwo),
                Some(MindObjective::Engage { target: Some(_) })
            ) && fired_at(lines, her_rig, intruder) > 0
        })
        .await;
    assert!(
        on_it,
        "the commander's own hull concentrates on the intruder: {:?}; {:?}",
        standing(&field.rig, okonkwo),
        orders_of(&mut field.rig, operation)
    );
    field.wait(6).await;

    // The absent player's unit: no order of hers reached it, the intruder never fired on it,
    // and it fired nothing, though the intruder is Hostile to its party and in its sight.
    assert!(
        orders_of(&mut field.rig, operation)
            .iter()
            .all(|held| held.slot == 2 || held.issuer != okonkwo),
        "an absent player's hull is sent no order: {:?}",
        orders_of(&mut field.rig, operation)
    );
    let given = crate::roster_rig::operation(&mut field.rig, operation)
        .and_then(|record| record.orders)
        .map(|orders| orders.given.clone())
        .expect("the roster's history");
    assert!(
        !given.is_empty() && given.iter().all(|order| order.slot.get() == 2),
        "every order the commander gave went to her own slot: {given:?}"
    );
    assert_eq!(fired_at(&field.lines, intruder, own_rig), 0);
    assert_eq!(
        fired(&field.lines, own_rig),
        0,
        "a unit flown for an absent player does not fire first"
    );
    assert!(
        tacsim::field::absent::minded(field.rig.tw.world(), own_rig)
            && tacsim::field::field_of_unit(field.rig.tw.world(), own_rig).is_some(),
        "it stands in the field, its mind's, the whole while"
    );
}

/// **Story 7** (this module's head): a restart with the NPC leading gives the same next order.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mw14c_a_restart_with_the_npc_leading_gives_the_same_next_order() {
    let at = &AHUNA_FLANK;
    let mut setup = at_the_ramp(at, npc_plan());
    setup.frames = false;
    let mut field = open(setup).await;
    let okonkwo = field.flown.operator(1);
    let lead_rig = field.flown.unit(1);
    let operation = field.flown.operation;

    // One world second at a time, to the second her first order is admitted: the advance to
    // her own hull. She owes the same to the other hull, and has not yet given it.
    let first = ticks(&mut field, 200, |rig| !orders_of(rig, operation).is_empty()).await;
    let book = orders_of(&mut field.rig, operation);
    assert!(
        first && book.len() == 1,
        "one order a fire: her first is admitted alone: {book:?}"
    );
    let own = book[0].clone();
    let Some(FieldObjective::Advance { to }) = own.objective else {
        panic!("her first order is the advance: {own:?}")
    };
    assert_eq!(
        (own.slot, own.issuer, own.issuer_unit),
        (1, okonkwo, lead_rig)
    );
    let stamped = issued_ic_ms(&mut field, 1, own.sequence).expect("its stamp");

    // The server goes down and comes up.
    let Field {
        rig,
        flown,
        other,
        stranger,
        escort,
        intruder,
        mut rx,
        ..
    } = field;
    let mut rig = restart(rig, &mut rx, &[other, stranger]).await;
    let mut rx = egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();
    assert!(
        until_composed(&mut rig, &mut rx, &mut lines, 30, |rig, _| task(rig)
            .is_some())
        .await,
        "the field resumes"
    );
    let mut field = Field {
        rig,
        flown,
        other,
        stranger,
        escort,
        intruder,
        rx,
        lines,
    };
    assert_eq!(
        field.leader(),
        Some((okonkwo, LeaderKind::Npc)),
        "she leads on the next boot"
    );

    // The next order is the one she owed: the same advance, to the other hull, next in the
    // book. Her own is not sent again, though the boot forgot what she had sent.
    let next = field
        .until(120, |rig, _| {
            orders_of(rig, operation).iter().any(|held| held.slot == 2)
        })
        .await;
    let book = orders_of(&mut field.rig, operation);
    assert!(next, "she gives the other hull its order: {book:?}");
    let other_hull = book
        .iter()
        .find(|held| held.slot == 2)
        .expect("slot 2's order");
    assert_eq!(
        (
            other_hull.objective.clone(),
            other_hull.issuer,
            other_hull.issuer_unit,
            other_hull.sequence
        ),
        (
            Some(FieldObjective::Advance { to }),
            okonkwo,
            lead_rig,
            own.sequence + 1
        ),
        "the same next order as the book owed before the restart: {book:?}"
    );
    field.wait(6).await;
    let book = orders_of(&mut field.rig, operation);
    let again: Vec<&Held> = book.iter().filter(|held| held.slot == 1).collect();
    assert!(
        again.len() == 1
            && again[0].sequence == own.sequence
            && issued_ic_ms(&mut field, 1, own.sequence) == Some(stamped),
        "her own hull's order is the one given before the restart, not a new one: {book:?}"
    );
    assert!(
        book.iter().all(|held| held.sequence <= own.sequence + 1),
        "and nothing else was ordered: {book:?}"
    );
}

/// **The orders a run delivers, as a log**: each delivered order's slot, sequence, issuer's
/// slot and words, in slot order. A contact is logged by its designator: its row key is the
/// commander's own scope's.
fn order_log(field: &mut Field) -> Vec<String> {
    let operation = field.flown.operation;
    let units = field.flown.units.clone();
    orders_of(&mut field.rig, operation)
        .into_iter()
        .filter(|held| held.delivered)
        .map(|held| {
            format!(
                "slot {} order {} from slot {:?}: {}",
                held.slot,
                held.sequence,
                units
                    .iter()
                    .position(|unit| *unit == held.issuer_unit)
                    .map(|index| index + 1),
                words(&held)
            )
        })
        .collect()
}

/// Story 5's script on a fresh world: the order log at the advance and at the `CONCENTRATE`.
async fn run_to_the_concentrate() -> (Vec<String>, Vec<String>) {
    let mut field = armed_roster().await;
    let (okonkwo, mendes) = (field.flown.operator(1), field.flown.operator(2));
    assert!(
        field
            .until(90, |rig, _| {
                [okonkwo, mendes]
                    .into_iter()
                    .all(|who| matches!(standing(rig, who), Some(MindObjective::Advance { .. })))
            })
            .await,
        "the advance stands"
    );
    let advance = order_log(&mut field);
    let _ = to_the_concentrate(&mut field).await;
    let log = order_log(&mut field);
    // The world ends here: its field's task is stopped before the next world starts.
    let tasks = field
        .rig
        .tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .map(|registry| registry.tasks().clone())
        .expect("the registry");
    tasks.shutdown(tacsim::arena_bridge::JOIN_TIMEOUT).await;
    (advance, log)
}

/// **Story 8** (this module's head): the same run twice gives the same orders. The commander
/// keeps no state but the roster's order book, and its rule reads no clock and no dice: the
/// dwell is read off the book's own stamps.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mw14c_the_same_run_gives_the_same_orders_twice() {
    let first = run_to_the_concentrate().await;
    let second = run_to_the_concentrate().await;
    assert_eq!(first, second);
    assert_eq!(first.0.len(), 2, "an advance a hull: {:?}", first.0);
    assert!(
        first.1.len() == 2 && first.1.iter().all(|line| line.contains("CONCENTRATE ")),
        "a CONCENTRATE a hull: {:?}",
        first.1
    );
}

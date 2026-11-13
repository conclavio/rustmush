//! **Doc 44 unit MW.14a — the compiled NPC commander** (Q-44-20, Q-44-44; doc 27 FO-15, Q-D4,
//! Q-F1), over the composed world (`driving`'s keyed rig) with the arena's egress pumped by hand,
//! the roster flown by `roster_rig::fly` (MW.15's rig).
//!
//! **The acceptance** ([`npc_led`], at 256 and at 512): a captain hires Sana Okonkwo under her
//! commander's contract and Ilya Mendes, names Okonkwo the plan's `npc_commander`, sails to the
//! site and launches both from aboard the carrier. No player is in the roster's units. An
//! escort from another captain and an intruder stand on the same ground, and the escort's fire
//! order promotes the site. Then, with nobody typing an order:
//!
//! 1. **advance**: nothing hostile is in Okonkwo's sight, so she orders both hulls to the work
//!    point; both orders are hers, from her unit, and both rigs drive for it;
//! 2. **a jammed order does not arrive**: staff jam the roster's comms (`@prop/jam`) and the
//!    intruder fires on the roster's carrier. Okonkwo's own frame now holds a Hostile and she
//!    orders HOLD, but the orders stay in flight on the transport: both rigs keep their ADVANCE;
//! 3. **hold**: the jam is cleared, the HOLD arrives, and both rigs stand;
//! 4. **the commander's hull is lost**: the intruder fires on Okonkwo's rig until it is
//!    destroyed. The field command passes to Mendes, the next NPC in the roster's order, under a
//!    new term;
//! 5. **withdraw**: Mendes orders the roster home, the order is his, from his unit, and his rig
//!    drives for the ramp.
//!
//! **A returning player takes the lead** ([`mw14a_a_returning_player_takes_the_lead`]): a
//! captain leads from rig 1 with Okonkwo, the plan's commander, in rig 2. While the captain
//! leads, Okonkwo orders nothing. The captain's session ends: Okonkwo leads and orders the
//! advance. The captain returns: after the roster's return hysteresis the lead is theirs, their
//! own `ORDER DT:… 2 HOLD` is delivered and obeyed, and Okonkwo gives no order under their term.
//!
//! **The same run twice** ([`mw14a_the_same_run_gives_the_same_orders_twice`]): the script to
//! the HOLD, run on two fresh worlds, delivers the same orders in the same order.
//!
//! What the stories stand in for:
//! - **the escort and the intruder are fixture hulls with players seated** (CF.2's `armed_on`
//!   and `seat`); the intruder's player types every shot (`LOCK`, `FIRE`);
//! - **the jam is staff's** (`@prop/jam <place>=4`): no player-held jammer reaches the IC
//!   transport yet;
//! - **the captain's link loss is the session's facts set by hand** (`set_connected`,
//!   `set_live_conns`), as `mush-server`'s
//!   `ic_field_leadership_uses_real_player_control_sessions_and_restart` sets them: what the
//!   field does with an absent pilot's hull is MW.7's, and is not exercised here.

use mush_core::ObjId;
use mush_tactics::detachment::LeaderKind;
use tacsim::arena::ledger::LedgerEvent;
use tacsim::arena::party::Relation;
use tacsim::arena::perception::MindObjective;
use tacsim::arena_task::ArenaOut;
use tacsim::detachment::sortie::FieldObjective;
use tokio::sync::mpsc::Receiver;

use crate::driving_rig::{
    Rig, cid, command, land, on_pad_as_at, ramp_at, run, stocked_tuned, warm,
};
use crate::field_host_rig::{
    armed_on, egress, frame, seat, second_composed, task, until_composed, weapon_of,
};
use crate::grounds_rig::{AHUNA_FLANK, Ground, URVARA_REACH};
use crate::roster_rig::{
    Billet, Flown, Held, Plan, commit, delivered_to, fence, field_roster, fly, orders_of,
};
use crate::site_record_rig::staff_member;

/// The shipped crew at Ceres (`content/crew/ceres.json`): the forewoman, under a commander's
/// contract, and the operator.
const OKONKWO: &str = "Sana Okonkwo";
const MENDES: &str = "Ilya Mendes";

/// **A field with a roster in it**: the roster, the two other parties and the arena's egress.
struct Field {
    rig: Rig,
    flown: Flown,
    /// The intruder's player and hull.
    stranger: ObjId,
    intruder: ObjId,
    rx: Receiver<ArenaOut>,
    lines: Vec<LedgerEvent>,
}

/// **Fly `plan`'s roster onto `at`'s ground and open a field over it**: the roster at its ramp,
/// an escort from another captain six cells east of the intruder, the intruder on the miner's
/// ground; the escort fires on the intruder's bearing and the site promotes with three parties.
async fn opened(at: &Ground, plan: &Plan) -> Field {
    let mut rig = stocked_tuned(|cfg| {
        crate::field_host_rig::tune(cfg);
        // The field holds through the story's quiet stretches.
        cfg.field.disengage_s = 3_600;
    });
    let flown = fly(&mut rig, at, plan, |_, _| {});
    let (captain, _, _) = on_pad_as_at(&mut rig, ("Other", 2, "Tally Moth"), at.site(), 0);
    let (stranger, _, _) = on_pad_as_at(&mut rig, ("Third", 3, "Lone Moth"), at.site(), 0);
    let escort = armed_on(
        &mut rig,
        at.entry,
        "Red Hound",
        (at.read.0 + 6, at.read.1),
        270_000,
    );
    let intruder = armed_on(&mut rig, at.entry, "Grey Hound", at.read, 270_000);
    seat(&mut rig, escort, captain);
    seat(&mut rig, intruder, stranger);
    warm(&mut rig);
    let mut rx = egress(&mut rig);
    let mut lines = Vec::new();
    for _ in 0..3 {
        let _ = second_composed(&mut rig);
    }
    let (out, queued) = command(&mut rig, captain, "+field/fire W");
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
    Field {
        rig,
        flown,
        stranger,
        intruder,
        rx,
        lines,
    }
}

/// The order `who`'s own frame shows standing.
fn standing(rig: &Rig, who: ObjId) -> Option<MindObjective> {
    frame(rig, who).and_then(|frame| frame.order.map(|order| order.objective))
}

/// Where `who`'s own frame puts its unit, centimetres.
fn at_cm(rig: &Rig, who: ObjId) -> Option<(i64, i64)> {
    frame(rig, who).map(|frame| (frame.pose.x_cm, frame.pose.y_cm))
}

/// How far `who`'s unit stands from a site cell's centre, centimetres.
fn range_to(rig: &Rig, who: ObjId, cell: (u16, u16)) -> Option<i64> {
    let (x, y) = at_cm(rig, who)?;
    let (dx, dy) = (
        x - (i64::from(cell.0) * 1_000 + 500),
        y - (i64::from(cell.1) * 1_000 + 500),
    );
    Some(i64::try_from((dx * dx + dy * dy).unsigned_abs().isqrt()).unwrap_or(i64::MAX))
}

/// The site cell `unit` stands on, by its surface placement.
fn cell_of(rig: &Rig, unit: ObjId) -> Option<(u16, u16)> {
    mush_sim::pose_of(rig.tw.world(), unit).and_then(|(_, pose)| {
        let (x, y) = tacsim::surface::excursion::site_cell(pose.x_cm, pose.y_cm);
        Some((u16::try_from(x).ok()?, u16::try_from(y).ok()?))
    })
}

/// **The cell the roster's carrier stands on in the field**, as `who`'s own frame holds it: the
/// contact of its own party that is none of the roster's `rigs`. Several hulls settle on one pad
/// and the field stands each on a cell of its own, so the carrier is named by its exact cell.
pub(crate) fn carrier_cell(rig: &Rig, who: ObjId, rigs: &[ObjId]) -> Option<(u16, u16)> {
    let own: Vec<(u16, u16)> = rigs
        .iter()
        .filter_map(|operator| frame(rig, *operator).and_then(|frame| frame.cell))
        .collect();
    frame(rig, who)?
        .contacts
        .iter()
        .filter(|contact| contact.relation == Relation::Own)
        .filter_map(|contact| contact.cell)
        .find(|cell| {
            own.iter()
                .all(|rig| rig.0.abs_diff(cell.0) > 1 || rig.1.abs_diff(cell.1) > 1)
        })
}

/// [`carrier_cell`], waited for: the commander's frame holds her own carrier within a few
/// pulses of the field's opening.
async fn carrier_pad(field: &mut Field, who: ObjId, rigs: &[ObjId]) -> (u16, u16) {
    let held = until_composed(
        &mut field.rig,
        &mut field.rx,
        &mut field.lines,
        30,
        |rig, _| carrier_cell(rig, who, rigs).is_some(),
    )
    .await;
    assert!(
        held,
        "the commander's frame holds her own carrier: {:?}",
        frame(&field.rig, who).map(|frame| frame
            .contacts
            .iter()
            .map(|contact| (contact.cell, contact.relation))
            .collect::<Vec<_>>())
    );
    carrier_cell(&field.rig, who, rigs).expect("held")
}

/// **The intruder's player keeps a lock on the contact within `within` cells of `cell` and
/// fires**, one pass of the two lines a player types (`escort_lease`'s exchange).
fn fire_on(field: &mut Field, cell: (u16, u16), within: u16) {
    let (stranger, gun) = (field.stranger, weapon_of(&field.rig, field.intruder));
    let near = frame(&field.rig, stranger).and_then(|frame| {
        frame
            .contacts
            .iter()
            .find(|contact| {
                contact.cell.is_some_and(|(x, y)| {
                    x.abs_diff(cell.0) <= within && y.abs_diff(cell.1) <= within
                })
            })
            .map(|contact| (contact.id, contact.designator.to_string(), frame.lock))
    });
    if let Some((id, designator, lock)) = near
        && lock != Some(id)
    {
        let _ = run(&mut field.rig.tw, stranger, &format!("LOCK {designator}"));
    }
    let _ = run(&mut field.rig.tw, stranger, &format!("FIRE {gun}"));
}

/// **What the intruder and `who` hold of each other**, for a failed wait: the intruder's lock,
/// its contacts and what its last two lines answered; `who`'s contacts and their relations.
fn picture(field: &mut Field, who: ObjId) -> String {
    let gun = weapon_of(&field.rig, field.intruder);
    let status = run(&mut field.rig.tw, field.stranger, "CONTACTS");
    let fire = run(&mut field.rig.tw, field.stranger, &format!("FIRE {gun}"));
    let rows = |rig: &Rig, of: ObjId| {
        frame(rig, of).map(|frame| {
            (
                frame.cell,
                frame.lock,
                frame
                    .contacts
                    .iter()
                    .map(|contact| {
                        (
                            contact.designator.to_string(),
                            contact.cell,
                            contact.relation,
                            contact.range_cm,
                        )
                    })
                    .collect::<Vec<_>>(),
            )
        })
    };
    format!(
        "intruder {:?}; CONTACTS: {status}; FIRE: {fire}; viewer {:?}; shots {}",
        rows(&field.rig, field.stranger),
        rows(&field.rig, who),
        field
            .lines
            .iter()
            .filter(|line| matches!(line.kind, tacsim::arena::ledger::EventKind::Fired { .. }))
            .count()
    )
}

/// The intruder's player ceases fire.
fn cease(field: &mut Field) {
    let gun = weapon_of(&field.rig, field.intruder);
    let _ = run(
        &mut field.rig.tw,
        field.stranger,
        &format!("FIRE {gun} OFF"),
    );
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

/// The roster of two NPCs under Okonkwo's command, launched by a captain who stays aboard.
fn npc_plan() -> Plan {
    Plan {
        who: ("Pilot", 1, "Salt Moth"),
        duty: "mining",
        billets: vec![Billet::Hired(OKONKWO), Billet::Hired(MENDES)],
        commander: Some(OKONKWO),
    }
}

/// **The field to the commander's ADVANCE** (the acceptance's step 1): the roster flown and the
/// field opened, both hulls holding Okonkwo's order to the work point; the field and the cell.
async fn advanced(at: &Ground) -> (Field, (u16, u16)) {
    let mut field = opened(at, &npc_plan()).await;
    let flown = &field.flown;
    let (okonkwo, mendes) = (flown.operator(1), flown.operator(2));
    let operation = flown.operation;
    // No player is in the roster's units, and the field command is Okonkwo's.
    let roster = field_roster(&mut field.rig, operation).expect("the field command");
    let leader = roster.leader().expect("a leader");
    assert_eq!(
        (leader.character, leader.kind),
        (okonkwo, LeaderKind::Npc),
        "the plan's NPC commander leads"
    );
    assert!(
        roster
            .members()
            .iter()
            .all(|member| member.controller.character() != Some(flown.organiser)),
        "the organiser launched the roster and is in none of its units"
    );
    let ramp = cell_of(&field.rig, flown.unit(2)).expect("the rig stands at the ramp");
    let ordered = until_composed(
        &mut field.rig,
        &mut field.rx,
        &mut field.lines,
        90,
        |rig, _| {
            [okonkwo, mendes]
                .into_iter()
                .all(|who| matches!(standing(rig, who), Some(MindObjective::Advance { .. })))
        },
    )
    .await;
    assert!(
        ordered,
        "the commander orders the advance: {:?} / {:?}; book {:?}",
        standing(&field.rig, okonkwo),
        standing(&field.rig, mendes),
        orders_of(&mut field.rig, operation)
    );
    let Some(MindObjective::Advance { to }) = standing(&field.rig, mendes) else {
        unreachable!("the advance stands")
    };
    assert_eq!(
        standing(&field.rig, okonkwo),
        Some(MindObjective::Advance { to }),
        "one work point for the roster"
    );
    let out = tacsim::detachment::commander::WORK_POINT_CELLS;
    let off = ramp.0.abs_diff(to.0).max(ramp.1.abs_diff(to.1));
    assert!(
        off.abs_diff(out) <= 1,
        "the work point lies {out} cells off the ramp at {ramp:?}: {to:?}"
    );
    (field, to)
}

/// The delivered order of `slot`, which must be `issuer`'s from `unit`.
fn held_by(field: &mut Field, slot: u16, issuer: ObjId, unit: ObjId) -> Held {
    let operation = field.flown.operation;
    let held = delivered_to(&mut field.rig, operation, slot)
        .unwrap_or_else(|| panic!("slot {slot} holds a delivered order"));
    assert_eq!(
        (held.issuer, held.issuer_unit),
        (issuer, unit),
        "slot {slot}'s order is its leader's, from the leader's unit: {held:?}"
    );
    held
}

/// **The acceptance on `at`'s ground** (this module's head).
#[allow(clippy::too_many_lines)] // one story, in order
async fn npc_led(at: &Ground) {
    // (1) Advance.
    let (mut field, to) = advanced(at).await;
    let (okonkwo, mendes) = (field.flown.operator(1), field.flown.operator(2));
    let (lead_rig, next_rig) = (field.flown.unit(1), field.flown.unit(2));
    let operation = field.flown.operation;
    for slot in [1, 2] {
        let held = held_by(&mut field, slot, okonkwo, lead_rig);
        assert_eq!(held.objective, Some(FieldObjective::Advance { to }));
    }
    let from = range_to(&field.rig, mendes, to).expect("the range to the work point");
    let closing = until_composed(
        &mut field.rig,
        &mut field.rx,
        &mut field.lines,
        90,
        |rig, _| {
            [okonkwo, mendes].into_iter().all(|who| {
                range_to(rig, who, to).is_some_and(|cm| cm + 1_500 <= from || cm <= 2_000)
            })
        },
    )
    .await;
    assert!(
        closing,
        "both rigs drive for the work point {to:?}: {:?} / {:?} from {from} cm",
        range_to(&field.rig, okonkwo, to),
        range_to(&field.rig, mendes, to)
    );
    let term = field_roster(&mut field.rig, operation)
        .expect("the field command")
        .term();

    // (2) The roster's comms are jammed, then a Hostile comes into the commander's sight: her
    // HOLD is admitted and sent, and does not arrive.
    let staff = staff_member(&mut field.rig);
    let places: std::collections::BTreeSet<ObjId> = {
        let world = field.rig.tw.world();
        let spatial = world
            .module_state::<mush_sim::SpatialState>()
            .expect("the spatial state");
        [okonkwo, mendes]
            .into_iter()
            .filter_map(|who| mush_propagation::context::place_of(spatial, world, who))
            .collect()
    };
    assert!(!places.is_empty(), "the crew are somewhere");
    for place in &places {
        let jammed = run(
            &mut field.rig.tw,
            staff,
            &format!("@prop/jam #{}=4", place.0),
        );
        assert!(jammed.contains("Interference at"), "{jammed}");
    }
    let pad = carrier_pad(&mut field, okonkwo, &[okonkwo, mendes]).await;
    let seen = until_shooting(&mut field, (pad, 0), 90, |rig| hostiles(rig, okonkwo) > 0).await;
    assert!(
        seen,
        "the intruder's fire on the carrier at {pad:?} puts a Hostile in her sight: {}",
        picture(&mut field, okonkwo)
    );
    cease(&mut field);
    let sent = until_composed(
        &mut field.rig,
        &mut field.rx,
        &mut field.lines,
        60,
        |rig, _| {
            orders_of(rig, operation)
                .iter()
                .filter(|held| !held.delivered && held.objective == Some(FieldObjective::Hold))
                .count()
                == 2
        },
    )
    .await;
    assert!(
        sent,
        "the commander's HOLD is in flight to both hulls: {:?}",
        orders_of(&mut field.rig, operation)
    );
    // Some fifteen world seconds on, neither has arrived: both hulls keep the order they had.
    wait(&mut field, 3).await;
    for (slot, who) in [(1, okonkwo), (2, mendes)] {
        let held = held_by(&mut field, slot, okonkwo, lead_rig);
        assert_eq!(
            held.objective,
            Some(FieldObjective::Advance { to }),
            "a jammed order does not arrive"
        );
        assert_eq!(
            standing(&field.rig, who),
            Some(MindObjective::Advance { to })
        );
    }

    // (3) The jam cleared, the HOLD arrives and both hulls stand.
    for place in &places {
        let _ = run(
            &mut field.rig.tw,
            staff,
            &format!("@prop/jam #{}=0", place.0),
        );
    }
    let held = until_composed(
        &mut field.rig,
        &mut field.rx,
        &mut field.lines,
        90,
        |rig, _| {
            [okonkwo, mendes].into_iter().all(|who| {
                standing(rig, who) == Some(MindObjective::Hold)
                    && frame(rig, who).is_some_and(|frame| frame.speed_cms == 0)
            })
        },
    )
    .await;
    assert!(
        held,
        "both hulls hold: {:?} / {:?}; book {:?}",
        standing(&field.rig, okonkwo),
        standing(&field.rig, mendes),
        orders_of(&mut field.rig, operation)
    );
    for slot in [1, 2] {
        let held = held_by(&mut field, slot, okonkwo, lead_rig);
        assert_eq!(held.objective, Some(FieldObjective::Hold));
    }
    assert_eq!(
        field_roster(&mut field.rig, operation)
            .expect("the field command")
            .term(),
        term,
        "one commander, one term, through the advance and the hold"
    );

    // (4) The commander's hull is lost: the next NPC in the roster's order leads.
    let target = frame(&field.rig, okonkwo)
        .and_then(|frame| frame.cell)
        .expect("the commander's cell");
    let passed = until_shooting(&mut field, (target, 1), 180, |rig| {
        frame(rig, okonkwo).is_some_and(|frame| frame.wreck)
    })
    .await;
    assert!(passed, "the commander's hull is destroyed");
    cease(&mut field);
    let succeeded = until_composed(
        &mut field.rig,
        &mut field.rx,
        &mut field.lines,
        90,
        |rig, _| {
            field_roster(rig, operation)
                .and_then(|roster| roster.leader())
                .is_some_and(|leader| leader.character == mendes)
        },
    )
    .await;
    let roster = field_roster(&mut field.rig, operation).expect("the field command");
    assert!(
        succeeded && roster.term() > term,
        "the next NPC leads under a new term: {:?}, term {} after {term}",
        roster.leader(),
        roster.term()
    );
    assert_eq!(
        roster.leader().map(|leader| leader.kind),
        Some(LeaderKind::Npc)
    );

    // (5) Withdraw: the successor's own order, from his own unit, and his rig makes for home.
    let ordered = until_composed(
        &mut field.rig,
        &mut field.rx,
        &mut field.lines,
        90,
        |rig, _| standing(rig, mendes) == Some(MindObjective::Withdraw),
    )
    .await;
    assert!(
        ordered,
        "the successor orders the roster home: {:?}; book {:?}",
        standing(&field.rig, mendes),
        orders_of(&mut field.rig, operation)
    );
    let held = held_by(&mut field, 2, mendes, next_rig);
    assert_eq!(held.objective, Some(FieldObjective::Withdraw));
    // The wrecked commander's slot holds the last order that reached it, and is given no other.
    let last = held_by(&mut field, 1, okonkwo, lead_rig);
    assert_eq!(last.objective, Some(FieldObjective::Hold));
    // Obeyed: the rig leaves where it held and closes on its carrier's ramp, or is already
    // through it and out of the field.
    let ramp = {
        let (x, y, _) = ramp_at(&field.rig.tw, at.entry);
        let (x, y) = tacsim::surface::excursion::site_cell(x, y);
        (
            u16::try_from(x).expect("a cell"),
            u16::try_from(y).expect("a cell"),
        )
    };
    let from = range_to(&field.rig, mendes, ramp).expect("the range home");
    let homing = until_composed(
        &mut field.rig,
        &mut field.rx,
        &mut field.lines,
        90,
        |rig, _| {
            tacsim::field::field_of_unit(rig.tw.world(), next_rig).is_none()
                || range_to(rig, mendes, ramp).is_some_and(|cm| cm + 1_500 <= from || cm <= 2_000)
        },
    )
    .await;
    assert!(
        homing,
        "the successor's rig makes for its ramp at {ramp:?}: {:?} from {from} cm",
        range_to(&field.rig, mendes, ramp)
    );
}

/// Pump and tick for `secs` of wall time (five world seconds to each).
async fn wait(field: &mut Field, secs: u64) {
    let _ = until_composed(
        &mut field.rig,
        &mut field.rx,
        &mut field.lines,
        secs,
        |_, _| false,
    )
    .await;
}

/// **Pump, tick and have the intruder's player fire on the contact at `cell`** until `done`
/// holds or `secs` of wall time pass (`field_host_rig::until_composed`, with the player's two
/// lines typed each world second).
async fn until_shooting(
    field: &mut Field,
    (cell, within): ((u16, u16), u16),
    secs: u64,
    mut done: impl FnMut(&Rig) -> bool,
) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    while std::time::Instant::now() < deadline {
        fire_on(field, cell, within);
        if until_composed(
            &mut field.rig,
            &mut field.rx,
            &mut field.lines,
            1,
            |rig, _| done(rig),
        )
        .await
        {
            return true;
        }
    }
    false
}

/// **MW.14a's acceptance at 256** on Ahuna Flank ([`npc_led`]).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mw14a_an_npc_led_detachment_advances_holds_and_withdraws() {
    npc_led(&AHUNA_FLANK).await;
}

/// **MW.14a's acceptance at 512** on Urvara Reach ([`npc_led`]).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mw14a_an_npc_led_detachment_advances_holds_and_withdraws_at_512() {
    npc_led(&URVARA_REACH).await;
}

/// **The orders a run delivers, as a log**: each delivered order's slot, sequence, issuer's slot
/// and objective, in slot order.
fn order_log(field: &mut Field) -> Vec<String> {
    let operation = field.flown.operation;
    let units = field.flown.units.clone();
    orders_of(&mut field.rig, operation)
        .into_iter()
        .filter(|held| held.delivered)
        .map(|held| {
            format!(
                "slot {} order {} from slot {:?}: {:?}",
                held.slot,
                held.sequence,
                units
                    .iter()
                    .position(|unit| *unit == held.issuer_unit)
                    .map(|index| index + 1),
                held.objective
            )
        })
        .collect()
}

/// **The script to the HOLD on a fresh world**: the roster flown, the field opened, the advance
/// delivered, the intruder's fire on the carrier, the hold delivered; the two order logs.
async fn to_the_hold(at: &Ground) -> (Vec<String>, Vec<String>) {
    let (mut field, _) = advanced(at).await;
    let (okonkwo, mendes) = (field.flown.operator(1), field.flown.operator(2));
    let advance = order_log(&mut field);
    let pad = carrier_pad(&mut field, okonkwo, &[okonkwo, mendes]).await;
    let seen = until_shooting(&mut field, (pad, 0), 90, |rig| hostiles(rig, okonkwo) > 0).await;
    assert!(seen, "the intruder's fire makes it Hostile in her sight");
    cease(&mut field);
    let held = until_composed(
        &mut field.rig,
        &mut field.rx,
        &mut field.lines,
        90,
        |rig, _| {
            [okonkwo, mendes]
                .into_iter()
                .all(|who| standing(rig, who) == Some(MindObjective::Hold))
        },
    )
    .await;
    assert!(held, "both hulls hold");
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

/// **MW.14a: the same run twice gives the same orders** (the row's last clause). Two fresh
/// worlds run the script to the HOLD; each delivers the same orders, with the same sequences,
/// from the same slot, in the same order. The commander keeps no state but the roster's order
/// book, and its rule reads no clock and no dice.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mw14a_the_same_run_gives_the_same_orders_twice() {
    let first = to_the_hold(&AHUNA_FLANK).await;
    let second = to_the_hold(&AHUNA_FLANK).await;
    assert_eq!(first, second);
    assert_eq!(first.0.len(), 2, "an advance a hull: {:?}", first.0);
    assert_eq!(first.1.len(), 2, "a hold a hull: {:?}", first.1);
}

/// **MW.14a: a returning player takes the lead** (this module's head; doc 27 FO-15, Q-D4).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // one story, in order
async fn mw14a_a_returning_player_takes_the_lead() {
    let at = &AHUNA_FLANK;
    let mut field = opened(
        at,
        &Plan {
            who: ("Pilot", 1, "Salt Moth"),
            duty: "mining",
            billets: vec![Billet::Leader, Billet::Hired(OKONKWO)],
            commander: Some(OKONKWO),
        },
    )
    .await;
    let captain = field.flown.organiser;
    let okonkwo = field.flown.operator(2);
    let (lead_rig, hired_rig) = (field.flown.unit(1), field.flown.unit(2));
    let operation = field.flown.operation;
    let leads = |rig: &mut Rig| {
        field_roster(rig, operation)
            .and_then(|roster| roster.leader())
            .map(|leader| leader.character)
    };
    assert_eq!(leads(&mut field.rig), Some(captain), "the player leads");

    // While a player leads, the plan's commander gives no order.
    wait(&mut field, 4).await;
    assert!(
        orders_of(&mut field.rig, operation).is_empty(),
        "the commander yields to the player who leads: {:?}",
        orders_of(&mut field.rig, operation)
    );
    assert_eq!(standing(&field.rig, okonkwo), None);

    // The captain's session ends: the commander leads, and orders the advance.
    field.rig.tw.world_mut().set_connected(captain, false);
    field.rig.tw.world_mut().set_live_conns(captain, 0);
    let took = until_composed(
        &mut field.rig,
        &mut field.rx,
        &mut field.lines,
        60,
        |rig, _| leads(rig) == Some(okonkwo),
    )
    .await;
    assert!(took, "the commander leads once no player is active");
    let ordered = until_composed(
        &mut field.rig,
        &mut field.rx,
        &mut field.lines,
        90,
        |rig, _| matches!(standing(rig, okonkwo), Some(MindObjective::Advance { .. })),
    )
    .await;
    assert!(
        ordered,
        "the commander orders the advance: {:?}; book {:?}",
        standing(&field.rig, okonkwo),
        orders_of(&mut field.rig, operation)
    );
    let hers = delivered_to(&mut field.rig, operation, 2).expect("her own order");
    assert_eq!((hers.issuer, hers.issuer_unit), (okonkwo, hired_rig));
    assert!(
        orders_of(&mut field.rig, operation)
            .iter()
            .all(|held| held.slot == 2),
        "an absent player's hull is sent no order: {:?}",
        orders_of(&mut field.rig, operation)
    );

    // The captain returns: after the return hysteresis the lead is theirs again.
    field.rig.tw.world_mut().set_connected(captain, true);
    field.rig.tw.world_mut().set_live_conns(captain, 1);
    let back = until_composed(
        &mut field.rig,
        &mut field.rx,
        &mut field.lines,
        60,
        |rig, _| leads(rig) == Some(captain),
    )
    .await;
    assert!(
        back,
        "the returning player takes the lead: {:?}",
        field_roster(&mut field.rig, operation).map(|roster| roster.leader())
    );
    // Their own order is delivered and obeyed, and the commander gives none under their term.
    let fenced = fence(&mut field.rig, captain, &field.flown);
    let _ = commit(&mut field.rig, captain, &format!("ORDER {fenced} 2 HOLD"));
    let held = until_composed(
        &mut field.rig,
        &mut field.rx,
        &mut field.lines,
        90,
        |rig, _| standing(rig, okonkwo) == Some(MindObjective::Hold),
    )
    .await;
    assert!(
        held,
        "the player's HOLD stands on the NPC's frame: {:?}; book {:?}",
        standing(&field.rig, okonkwo),
        orders_of(&mut field.rig, operation)
    );
    let theirs = delivered_to(&mut field.rig, operation, 2).expect("the player's order");
    assert_eq!((theirs.issuer, theirs.issuer_unit), (captain, lead_rig));
    let sequence = theirs.sequence;
    wait(&mut field, 4).await;
    let book = orders_of(&mut field.rig, operation);
    assert!(
        book.iter().all(|held| held.sequence <= sequence)
            && standing(&field.rig, okonkwo) == Some(MindObjective::Hold),
        "the commander gives no order while the player leads: {book:?}"
    );
}

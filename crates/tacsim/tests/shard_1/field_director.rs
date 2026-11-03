//! **Doc 36 cut GC.11 — the raid director** (Q-36-41, Q-36-55), over the composed world
//! (`driving`'s keyed rig on Ahuna Flank), the arena's egress pumped by hand as
//! `field_raiders.rs` pumps it.
//!
//! The gate: the director lands the content's raider on a site where three flights are out and a
//! rig holds ore, its leader declares through the ordinary door, the field runs, the site's guard
//! takes the raid's worn hull, and the raid, having lost half of what it launched, turns for
//! home: its standing hull goes up the ramp and aboard, its carrier lifts by GC.0's lift, and
//! what it lost stays where it fell. The raider's organization stands in the faction registry
//! with its leader as principal, and the cooldown holds the next raid back though the site still
//! tempts it.
//!
//! Beside it, the arrival: a raid scheduled while a field already runs on the site arrives by the
//! kernel's join (GC.1b) — its carrier and hulls rows of the raider's own party, its carrier's
//! ramp laid as that party's home — and, once it has spent `raid_field_s` there (its guard
//! carries no rounds, so it loses nothing), goes home up that ramp.
//!
//! **What is a fixture** (doc 36a lane 4). The site the director reads is seeded by staff hand:
//! the ore in a rig's hopper (`seed_pool`), the guard's hull (`armed`), the hired NPC in its seat
//! (`seat_npc`) and its magazines rationed (`rationed`, to `ROUNDS`). Both gates seed the raider
//! itself by staff before the raid (`raiders::seed`) and lift its carrier (`unplace`), so it
//! stands as one that has raided before and lifted, and the director sends it as it is. In the
//! first gate one of its hulls came home from that earlier raid worn thin (`fragile`, armour at
//! its floor): it is the one the guard's fire engages first, the guard's six rounds take it, and
//! nothing of the raider's is touched once the director has landed it. A whole line tank is not
//! what the gate kills: lane 4 gave the guard full magazines against two whole hulls, and in
//! 73 s of fire it wrecked only the raider's carrier, never a hull, then fell; the raiders had
//! wrecked both rigs and the prospector's carrier first, and with its own carrier gone the raid
//! could not turn for home. So the loss the gate earns is the worn hull's.

use mush_core::{GOD, ObjId, ObjKind};
use mush_inventory::LotId;
use tacsim::TacsimConfig;
use tacsim::arena::ledger::LedgerEvent;
use tacsim::carrier_damage::CarrierLoss;
use tacsim::field::PartyKey;
use tacsim::surface::site_record::{LeftCondition, PromotedBy};

use crate::driving::{
    AHUNA, Rig, act_go, cid, command, finish, land, launched, on_pad, seed_pool, staff_hand,
    stocked_tuned, to_cell, warm,
};
use crate::field_gc3::set_section;
use crate::field_raiders::{
    SKULKS, armed, egress, fragile, raider_hulls, seat_npc, second, site_record, task, until,
};

/// A fast field, and the director on.
fn tune(cfg: &mut TacsimConfig) {
    cfg.arena.tempo.quiet_ms = 100;
    cfg.arena.tempo.alert_ms = 100;
    cfg.arena.tempo.engaged_ms = 100;
    cfg.arena.snapshot_ms = 500;
    cfg.field.disengage_s = 10;
    cfg.field.raid_director = true;
}

/// The content raider's carrier, once the director has seeded it.
fn carrier(rig: &Rig) -> Option<ObjId> {
    raider_hulls(rig)
        .first()
        .and_then(|hull| tacsim::field::raiders::carrier_of(rig.tw.world(), *hull))
}

/// The raider's carrier, seeded (on the ground or not).
fn carrier_seeded(rig: &Rig) -> ObjId {
    raider_hulls(rig)
        .first()
        .and_then(|hull| tacsim::field::raiders::carrier_of(rig.tw.world(), *hull))
        .expect("the raider is seeded")
}

/// Three tonnes of ore into `tv`'s hopper.
fn ore(rig: &mut Rig, tv: ObjId) {
    let hopper = tacsim::port::vehicle::hopper_of(rig.tw.world(), tv).expect("a hopper");
    seed_pool(rig, hopper, "metal_ore", 3, LotId(9_911));
}

/// **The site the director reads**: a prospector's carrier on the pad, its two rigs out east of
/// it, and an armed hull a hired NPC sits in further east — three flights out — and, with
/// `loaded`, rig 1's hopper holding three tonnes of ore. Answers the pilot, the rigs and the guard.
fn tempting_site(rig: &mut Rig, loaded: bool) -> (ObjId, Vec<ObjId>, ObjId) {
    let (a, _, tvs) = on_pad(rig, 2);
    launched(rig, a, tvs[0]);
    to_cell(rig, tvs[0], (190, 126));
    let _ = act_go(rig, a, "+tv/2 launch go");
    let _ = finish(rig, tvs[1], a);
    to_cell(rig, tvs[1], (190, 132));
    if loaded {
        ore(rig, tvs[0]);
    }
    let guard = armed(rig, "Ward Dog", (183, 126), 270_000);
    let npc = staff_hand(rig, |ctx| {
        ctx.w
            .create(ObjKind::Npc, "Hired guard", GOD, None)
            .expect("the NPC")
    });
    seat_npc(rig, guard, npc);
    rationed(rig, guard, ROUNDS);
    warm(rig);
    (a, tvs, guard)
}

/// The rounds the guard carries in each magazine: enough to take a worn hull (its armour at the
/// floor, a 105 mm round's 120 against internals of 200 or less), never a whole one (a facet of
/// 400 before them) or the raider's carrier.
const ROUNDS: i64 = 6;

/// **Ration `unit`'s magazines** to at most `rounds` each (a staff hand).
fn rationed(rig: &mut Rig, unit: ObjId, rounds: i64) {
    staff_hand(rig, |ctx| {
        let cells = mush_tactics::module::unit_of(ctx.w, unit)
            .expect("the unit")
            .clone();
        let ops: Vec<mush_tactics::UnitMutationOp> = cells
            .cells()
            .filter(|(addr, _)| {
                matches!(addr, mush_tactics::CellAddr::Component { cell, .. }
                    if cell.as_str() == tacsim::definitions::unit::AMMO_CELL)
            })
            .map(|(addr, cell)| mush_tactics::UnitMutationOp::SetCell {
                addr: addr.clone(),
                value: cell.current.min(rounds).max(cell.min),
            })
            .collect();
        let plan = mush_tactics::UnitMutationPlan::new(
            ops,
            None,
            Some(smol_str::SmolStr::new_static("test:gc11-rationed")),
        )
        .expect("a plan");
        mush_tactics::module::apply_plan(ctx, unit, &plan).expect("applied");
    });
}

/// Every kill in the ledger: who, by whom, when.
fn kills(lines: &[LedgerEvent]) -> Vec<(ObjId, Option<ObjId>, i64)> {
    lines
        .iter()
        .filter_map(|line| match line.kind {
            tacsim::arena::ledger::EventKind::UnitDestroyed { unit, by } => {
                Some((unit, by, line.t_ms))
            }
            _ => None,
        })
        .collect()
}

/// Every shot `shooter` fired: at whom, how many rounds, when.
fn fire(lines: &[LedgerEvent], shooter: ObjId) -> Vec<(ObjId, u32, i64)> {
    lines
        .iter()
        .filter_map(|line| match &line.kind {
            tacsim::arena::ledger::EventKind::Fired {
                unit,
                target,
                rounds,
                ..
            } if *unit == shooter => Some((*target, *rounds, line.t_ms)),
            _ => None,
        })
        .collect()
}

/// Every hit `shooter` scored: on whom, which section, when.
fn hits(lines: &[LedgerEvent], shooter: ObjId) -> Vec<(ObjId, String, i64)> {
    lines
        .iter()
        .filter_map(|line| match &line.kind {
            tacsim::arena::ledger::EventKind::Hit {
                unit,
                target,
                section,
                ..
            } if *unit == shooter => Some((*target, section.to_string(), line.t_ms)),
            _ => None,
        })
        .collect()
}

/// **The raider as one that has raided before** (this file's head): seeded by staff, one hull
/// worn thin when `worn`, its carrier lifted off the ground; the director sends it as it is.
/// Answers its carrier and its hulls.
fn raided_before(rig: &mut Rig, worn: bool) -> (ObjId, Vec<ObjId>) {
    let def = tacsim::field::raiders::raiders()
        .expect("the fixture raiders")
        .into_iter()
        .find(|def| def.id.as_str() == SKULKS)
        .expect("the content's raider");
    let seeded = staff_hand(rig, |ctx| tacsim::field::raiders::seed(ctx, &def));
    assert!(seeded.is_ok(), "{seeded:?}");
    let barge = carrier_seeded(rig);
    let raiders = raider_hulls(rig);
    assert_eq!(raiders.len(), 2, "the content's two hulls");
    // The second hull is the one the site's guard's fire engages first.
    if worn {
        fragile(rig, raiders[1]);
    }
    staff_hand(rig, |ctx| {
        mush_sim::unplace(ctx, barge, "test: lifted").expect("lifted");
    });
    (barge, raiders)
}

/// **The GC.11 gate** (doc 36 §7; Q-36-41, Q-36-55).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // one raid, in order
async fn gc11_a_scheduled_raid_arrives_declares_and_leaves_with_its_losses() {
    let mut rig = stocked_tuned(tune);
    // The raider has raided before: one hull came home worn thin, and its carrier lifted.
    let (barge, raiders) = raided_before(&mut rig, true);
    assert!(
        mush_sim::pose_of(rig.tw.world(), barge).is_none(),
        "the raider's carrier is off the ground until the director sends it"
    );
    let (a, _, guard) = tempting_site(&mut rig, true);
    let mut rx = egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();

    // The director lands it: its carrier down at its cell, both hulls beside the ramp.
    let landed = until(&mut rig, &mut rx, &mut lines, 20, |rig, _| {
        mush_sim::pose_of(rig.tw.world(), barge).is_some()
            && raiders
                .iter()
                .all(|hull| mush_sim::pose_of(rig.tw.world(), *hull).is_some())
    })
    .await;
    assert!(landed, "the director lands the raid on its own");
    assert_eq!(carrier(&rig), Some(barge));
    // The wear came through the landing: the worn hull's armour is at its floor.
    let armour = |rig: &Rig, unit: ObjId| -> Vec<(String, i64, i64)> {
        mush_tactics::module::unit_of(rig.tw.world(), unit)
            .expect("the unit")
            .cells()
            .filter(|(addr, _)| {
                matches!(addr, mush_tactics::CellAddr::Facet { cell, .. }
                    if cell.as_str() == tacsim::definitions::unit::ARMOR_CELL)
            })
            .map(|(addr, cell)| (format!("{addr:?}"), cell.current, cell.min))
            .collect()
    };
    let worn = armour(&rig, raiders[1]);
    let whole = armour(&rig, raiders[0]);
    assert!(
        worn.iter().all(|(_, current, min)| current == min),
        "the worn hull's armour is at its floor after the landing: worn {worn:?}; whole {whole:?}; poses {:?}",
        raiders
            .iter()
            .map(|hull| mush_sim::pose_of(rig.tw.world(), *hull))
            .collect::<Vec<_>>()
    );
    assert!(mush_sim::pose_of(rig.tw.world(), barge).is_some());
    assert!(tacsim::surface::raid::director_raid(rig.tw.world(), barge));
    let landed_ic = tacsim::surface::raid::last_raid_ic(rig.tw.world(), barge)
        .expect("the landing is stamped for the cooldown");
    let leader = tacsim::field::raiders::leader_of(rig.tw.world(), barge).expect("a leader");
    // Its organization is in the registry, its leader the principal (Q-36-55).
    let organization = rig
        .tw
        .world()
        .module_state::<mush_faction::FactionState>()
        .and_then(|state| state.faction_by_name("Skulk Syndicate"))
        .cloned()
        .expect("the raider's organization is registered");
    assert_eq!(organization.principals, vec![leader]);

    // The leader declares through the ordinary door, and the field opens on its mark.
    let opened = until(&mut rig, &mut rx, &mut lines, 20, |rig, _| {
        task(rig).is_some()
    })
    .await;
    assert!(opened, "the director's declaration opens a field");
    let record = site_record(&mut rig).expect("the record");
    assert!(
        matches!(record.promoted.as_ref().map(|open| &open.by),
            Some(PromotedBy::Mark { leader: by, .. }) if *by == leader),
        "promoted by the raider leader's mark: {:?}",
        record.promoted
    );
    rig.tw.world_mut().set_connected(a, false);
    let live = tacsim::field::live_on(rig.tw.world(), &cid(AHUNA)).expect("the live record");
    let party = live.party_of(barge).expect("the carrier is in the field");
    assert!(
        live.parties
            .iter()
            .any(|p| p.party == party && p.key == PartyKey::Fixture { leader }),
        "the raid is its leader's party"
    );

    // The guard's fire takes the worn hull, which is half of what launched: the raid turns for
    // home, its standing hull boards, and its carrier lifts. Which of the two falls is read off
    // the record, not assumed.
    let aboard = |rig: &Rig| {
        raiders
            .iter()
            .filter(|hull| mush_sim::pose_of(rig.tw.world(), **hull).is_none())
            .count()
    };
    let left = until(&mut rig, &mut rx, &mut lines, 300, |rig, _| {
        mush_sim::pose_of(rig.tw.world(), barge).is_none() && aboard(rig) == 1
    })
    .await;
    let record = site_record(&mut rig).expect("the record");
    assert!(
        left,
        "the raid leaves with its losses; kills {:?}; the guard fired {:?} and hit {:?}; record units {:?}, withdrawing {:?}, aboard {}, guard standing {}",
        kills(&lines),
        fire(&lines, guard),
        hits(&lines, guard),
        record.units,
        raiders
            .iter()
            .map(|hull| tacsim::field::raiders::withdrawing(rig.tw.world(), *hull))
            .collect::<Vec<_>>(),
        aboard(&rig),
        !kills(&lines).iter().any(|(unit, _, _)| *unit == guard)
    );
    // What it lost stays where it fell; the one that went home is aboard, not on the record.
    let settled = until(&mut rig, &mut rx, &mut lines, 60, |rig, _| {
        site_record(rig).is_some_and(|record| record.promoted.is_none())
    })
    .await;
    assert!(settled, "the field settles once the carrier has gone");
    let record = site_record(&mut rig).expect("the record");
    let lost: Vec<ObjId> = record
        .units
        .iter()
        .filter(|unit| {
            raiders.contains(&unit.unit)
                && matches!(
                    unit.condition,
                    LeftCondition::Casualty | LeftCondition::Wreck
                )
        })
        .map(|unit| unit.unit)
        .collect();
    assert_eq!(
        lost.len(),
        1,
        "the guard took one hull, and it stays where it fell: {:?}; kills {:?}",
        record.units,
        kills(&lines)
    );
    let home = raiders
        .iter()
        .copied()
        .find(|hull| *hull != lost[0])
        .expect("the other hull");
    assert!(
        record.units.iter().all(|unit| unit.unit != home),
        "the hull that went home is aboard: {:?}",
        record.units
    );
    assert!(mush_sim::pose_of(rig.tw.world(), lost[0]).is_some());

    // The raid is over, and the cooldown holds the next one back though the site still tempts it.
    for _ in 0..10 {
        second(&mut rig);
    }
    assert!(
        !tacsim::surface::raid::director_raid(rig.tw.world(), barge),
        "the raid is over"
    );
    assert!(
        mush_sim::pose_of(rig.tw.world(), barge).is_none(),
        "the cooldown keeps the carrier away"
    );
    assert_eq!(
        tacsim::surface::raid::last_raid_ic(rig.tw.world(), barge),
        Some(landed_ic)
    );
    assert!(!tacsim::surface::raid::spent(rig.tw.world(), barge));
}

/// **A raid arriving in a running field** (doc 36 cut GC.11 on GC.1b's join): a field is open on
/// the site when the director's schedule comes due, so the carrier and its hulls join it as the
/// raider's party, with the carrier's ramp laid as that party's home; once it has spent its time
/// in the field it turns for home, every hull still standing goes home up that ramp, and its
/// carrier lifts.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // one raid, in order
async fn gc11_a_raid_due_while_a_field_runs_arrives_by_the_join_and_goes_home() {
    let mut rig = stocked_tuned(|cfg| {
        tune(cfg);
        // The field the raid arrives in stays open on the guard's mark until it does, and the
        // raid's time in the field is short: it turns for home after a minute there.
        cfg.field.disengage_s = 600;
        cfg.field.raid_field_s = 60;
    });
    // No ore aboard yet, so the director waits. The guard carries no rounds here: it holds the
    // field open as an armed party without wrecking the raider's carrier, which never lifts, so
    // this raid loses nothing and leaves on `raid_field_s`.
    let (a, tvs, guard) = tempting_site(&mut rig, false);
    rationed(&mut rig, guard, 0);
    // The raider has stood before and lifted: seeded, its carrier off the ground, nothing under
    // way, both hulls whole. The director sends it as it is.
    let (barge, raiders) = raided_before(&mut rig, false);
    let mut rx = egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();
    // The prospector marks the guard, west of its rigs, Hostile: a field opens before the raid.
    let (out, queued) = command(&mut rig, a, "+field/mark W=hostile");
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    land(&mut rig, &queued[0]).expect("the promotion lands");
    let opened = until(&mut rig, &mut rx, &mut lines, 20, |rig, _| {
        task(rig).is_some()
    })
    .await;
    assert!(opened, "a field runs on the site");
    assert!(
        mush_sim::pose_of(rig.tw.world(), barge).is_none(),
        "no raid before the ore"
    );
    // Ore comes aboard rig 1 while the field runs: the raid is due.
    ore(&mut rig, tvs[0]);

    // The director's raid arrives by the join.
    let joined = until(&mut rig, &mut rx, &mut lines, 30, |rig, _| {
        tacsim::field::live_on(rig.tw.world(), &cid(AHUNA)).is_some_and(|live| {
            live.units
                .iter()
                .filter(|row| row.arrived)
                .filter(|row| {
                    row.unit == barge
                        || tacsim::field::raiders::carrier_of(rig.tw.world(), row.unit)
                            == Some(barge)
                })
                .count()
                == 3
        })
    })
    .await;
    assert!(joined, "the carrier and both hulls join the running field");
    let live = tacsim::field::live_on(rig.tw.world(), &cid(AHUNA)).expect("the live record");
    let party = live.party_of(barge).expect("a party");
    for hull in &raiders {
        assert_eq!(live.party_of(*hull), Some(party), "one raider party");
    }
    let ramp = tacsim::field::raiders::ramp_cell(rig.tw.world(), barge).expect("a ramp");
    assert!(
        live.homes.iter().any(|home| home.team == party
            && (i64::from(home.x0), i64::from(home.y0)) == (ramp[0], ramp[1])),
        "the arriving party's home is its carrier's ramp: {:?}",
        live.homes
    );

    // Its field time spent (`raid_field_s`), it turns for home and goes up its ramp; its carrier
    // lifts. Each standing hull's exit at the ramp is named on the open promotion while the field
    // runs.
    let mut exited: Vec<ObjId> = Vec::new();
    let left = until(&mut rig, &mut rx, &mut lines, 240, |rig, _| {
        if let Some(record) = site_record(rig) {
            for hull in &raiders {
                if record.exited(*hull) && !exited.contains(hull) {
                    exited.push(*hull);
                }
            }
        }
        !tacsim::surface::raid::director_raid(rig.tw.world(), barge)
            && mush_sim::pose_of(rig.tw.world(), barge).is_none()
    })
    .await;
    let record = site_record(&mut rig).expect("the record");
    assert!(
        left,
        "the arrived raid goes home; kills {:?}; exits {:?}, withdrawing {:?}",
        kills(&lines),
        record.promoted.as_ref().map(|open| open.exits.clone()),
        raiders
            .iter()
            .map(|hull| tacsim::field::raiders::withdrawing(rig.tw.world(), *hull))
            .collect::<Vec<_>>()
    );
    // It turned for home: every hull that did not fall went up the ramp and out of the field.
    let fell: Vec<ObjId> = kills(&lines).iter().map(|(unit, _, _)| *unit).collect();
    assert!(!exited.is_empty(), "a hull left the field at its ramp");
    for hull in &raiders {
        assert!(
            exited.contains(hull) || fell.contains(hull),
            "#{} went home or fell: exits {exited:?}, kills {:?}",
            hull.0,
            kills(&lines)
        );
    }
}

/// **Doc 36a lane 3, P1-11** (doc 36 cut GC.3's rule, for the director's own barge): a barge
/// whose drive is down stays on the ground. The director lands its raid as the GC.11 gate has
/// it; staff then destroy the barge's drive where a round would have. The raid loses half, turns
/// for home and its standing hull boards — and the barge, which cannot lift, stays in the field
/// until it settles, a casualty on the index. The director reads it stuck: the raid is over, the
/// raider spent, and no visit afterwards takes the barge off the ground or its row off the record
/// (before the fix `recall` lifted it anyway and left the row behind).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // one raid, in order
async fn lane3_a_barge_whose_drive_is_down_stays_on_the_ground() {
    // The raid turns for home on its time in the field (the band's floor), whatever the guard's
    // rounds do: the gate is the barge's lift, not the fight.
    let mut rig = stocked_tuned(|cfg| {
        tune(cfg);
        cfg.field.raid_field_s = 60;
    });
    let (a, _, _) = tempting_site(&mut rig, true);
    let mut rx = egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();
    let landed = until(&mut rig, &mut rx, &mut lines, 20, |rig, _| {
        let hulls = raider_hulls(rig);
        hulls.len() == 2
            && hulls
                .iter()
                .all(|hull| mush_sim::pose_of(rig.tw.world(), *hull).is_some())
    })
    .await;
    assert!(landed, "the director lands the raid on its own");
    let raiders = raider_hulls(&rig);
    let barge = carrier(&rig).expect("the raider's carrier");
    // The drive as a field leaves it (GC.3's fixture): destroyed, its internals at their floor.
    set_section(&mut rig, barge, "drive", Some("destroyed"), Some(0));
    let damage = tacsim::carrier_damage::damage_of(rig.tw.world(), barge).expect("a reading");
    assert!(damage.has(CarrierLoss::RefusesLift) && !damage.has(CarrierLoss::Wrecked));
    fragile(&mut rig, raiders[0]);

    let opened = until(&mut rig, &mut rx, &mut lines, 20, |rig, _| {
        task(rig).is_some()
    })
    .await;
    assert!(opened, "the director's declaration opens a field");
    rig.tw.world_mut().set_connected(a, false);

    // It turns for home — on its losses or its time — and its standing hulls board; the barge
    // cannot follow.
    let boarded = until(&mut rig, &mut rx, &mut lines, 300, |rig, _| {
        raiders.iter().all(|hull| {
            mush_sim::pose_of(rig.tw.world(), *hull).is_none()
                || site_record(rig)
                    .is_some_and(|record| record.units.iter().any(|left| left.unit == *hull))
        })
    })
    .await;
    assert!(
        boarded,
        "the standing hull goes home up the ramp; kills {:?}",
        kills(&lines)
    );
    assert!(
        mush_sim::pose_of(rig.tw.world(), barge).is_some(),
        "the barge cannot lift"
    );
    let settled = until(&mut rig, &mut rx, &mut lines, 120, |rig, _| {
        site_record(rig).is_some_and(|record| record.promoted.is_none())
    })
    .await;
    assert!(settled, "the field settles with the barge on the ground");
    let record = site_record(&mut rig).expect("the record");
    assert!(
        record
            .units
            .iter()
            .any(|unit| unit.unit == barge && unit.condition == LeftCondition::Casualty),
        "the barge is a casualty on the index: {:?}",
        record.units
    );

    // The director's visits afterwards: the raid is over, the raider spent, the barge stays.
    for _ in 0..10 {
        second(&mut rig);
    }
    assert!(
        mush_sim::pose_of(rig.tw.world(), barge).is_some(),
        "a barge whose drive is down stays on the ground"
    );
    let record = site_record(&mut rig).expect("the record");
    assert!(
        record.units.iter().any(|unit| unit.unit == barge),
        "its row stands: {:?}",
        record.units
    );
    assert!(
        !tacsim::surface::raid::director_raid(rig.tw.world(), barge),
        "the raid is over"
    );
    assert!(
        tacsim::surface::raid::spent(rig.tw.world(), barge),
        "the director never sends it again"
    );
}

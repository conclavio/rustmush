//! **Doc 32 cut CF.6 — the fixture raider party and the NPC seat on doctrine**, over the
//! composed world (`driving`'s keyed rig on Ahuna Flank), the arena's egress pumped by hand as
//! `field_host.rs` pumps it.
//!
//! The gate's last statement: a raider party authored as content is seeded on a site, its leader
//! marks a party there Hostile through the ordinary door, and the field runs with no player
//! connected — the raider's hulls flying their ROM doctrine under their leader's standing marks,
//! a hired NPC's armed hull beside a prospector flying the doctrine too (its seat kept, its hull
//! the mind's) and answering fire once it is fired on — until the raider is destroyed and the
//! field demotes on its own.

use std::time::Duration;

use mush_core::{GOD, ObjId, ObjKind};
use tacsim::TacsimConfig;
use tacsim::arena::ledger::{EventKind, LedgerEvent};
use tacsim::arena_task::ArenaOut;
use tacsim::field::PartyKey;
use tacsim::surface::site_record::{LeftCondition, SiteKey, SiteRecord};
use tokio::sync::mpsc::Receiver;

use crate::driving::{
    AHUNA, Rig, ahuna_domain, cid, command, drain, handles, land, launched, on_pad, run,
    staff_hand, stocked_tuned, to_cell, typed, warm,
};

/// The raider the content ships.
pub(crate) const SKULKS: &str = "tacsim.raider.ahuna_skulks";

/// A fast field: the lab tempo, a short quiet interval before demotion.
fn tune(cfg: &mut TacsimConfig) {
    cfg.arena.tempo.quiet_ms = 100;
    cfg.arena.tempo.alert_ms = 100;
    cfg.arena.tempo.engaged_ms = 100;
    cfg.arena.snapshot_ms = 500;
    cfg.field.disengage_s = 10;
}

/// Ahuna Flank's record.
fn site_record(rig: &mut Rig) -> Option<SiteRecord> {
    let h = handles(&rig.tw);
    typed(&mut rig.tw, &h.families.sites, &SiteKey(cid(AHUNA)))
}

/// **An armed hull** (the line tank) on Ahuna Flank's ground at `cell`.
fn armed(rig: &mut Rig, name: &str, cell: (i64, i64), heading: u32) -> ObjId {
    let domain = ahuna_domain(&rig.tw);
    staff_hand(rig, |ctx| {
        let spec = ctx
            .w
            .module_state::<tacsim::TacsimStore>()
            .and_then(|store| {
                store
                    .definitions()
                    .bindings
                    .get(&cid("tacsim.unit.line_tank"))
                    .map(|binding| binding.spec.clone())
            })
            .expect("the line tank is bound");
        let obj = ctx
            .w
            .create(ObjKind::Thing, name, GOD, None)
            .expect("the hull's object");
        mush_tactics::module::create_unit(ctx, obj, &spec).expect("the hull's unit record");
        let pose =
            tacsim::surface::excursion::cell_pose([cell.0, cell.1], heading).expect("a site pose");
        mush_sim::place(ctx, obj, domain, pose).expect("placed on the ground");
        obj
    })
}

/// **An NPC in `unit`'s cockpit station**, every cockpit channel granted.
pub(crate) fn seat_npc(rig: &mut Rig, unit: ObjId, npc: ObjId) {
    staff_hand(rig, |ctx| {
        ctx.actor = npc;
        let station = mush_sim::StationId::new("cockpit").expect("a station id");
        let channels = tacsim::match_::cockpit::cockpit_channels();
        mush_sim::declare_station(
            ctx,
            unit,
            &station,
            tacsim::match_::cockpit::STATION_ROLE,
            &channels,
            None,
            tacsim::match_::cockpit::STATION_PRIORITY,
        )
        .expect("the station");
        // The billet alone, as a staff hand crews it: the NPC sits at the station.
        mush_sim::commands::assign_station(ctx, unit, &station, npc).expect("the seat");
    });
}

/// **Make `unit` fragile**: armour at its floor, internals one above, so a few hits wreck it.
fn fragile(rig: &mut Rig, unit: ObjId) {
    staff_hand(rig, |ctx| {
        let cells = mush_tactics::module::unit_of(ctx.w, unit)
            .expect("the unit")
            .clone();
        let ops: Vec<mush_tactics::UnitMutationOp> = cells
            .cells()
            .filter_map(|(addr, cell)| {
                let name = match addr {
                    mush_tactics::CellAddr::Facet { cell, .. }
                    | mush_tactics::CellAddr::Section { cell, .. } => cell.as_str(),
                    _ => return None,
                };
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
            Some(smol_str::SmolStr::new_static("test:cf6-fragile")),
        )
        .expect("a plan");
        mush_tactics::module::apply_plan(ctx, unit, &plan).expect("applied");
    });
}

/// The registry's egress, claimed once per process.
fn egress(rig: &mut Rig) -> Receiver<ArenaOut> {
    rig.tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .map(|registry| registry.tasks().clone())
        .and_then(|tasks| tasks.take_egress())
        .expect("the registry's egress")
}

/// The world loop's `Wake::ArenaEgress` arm, by hand, keeping every ledger line.
fn pump(rig: &mut Rig, rx: &mut Receiver<ArenaOut>, lines: &mut Vec<LedgerEvent>) {
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
        rig.tw.apply(mush_core::WorldCommand::ModuleAsyncDone {
            module: smol_str::SmolStr::new_static(tacsim::arena_bridge::MODULE),
            actor: GOD,
            correlation: 0,
            payload: tacsim::arena_bridge::EGRESS_PAYLOAD.to_owned(),
        });
        tasks.stamp_ledger_writes(1);
        tasks.ack_ledger_upto(1);
    }
}

/// One world second: the ticks due, and every submission they queued, landed.
fn second(rig: &mut Rig) {
    let t = rig.tw.world().now().saturating_add(1);
    rig.tw.set_now(t);
    let _ = rig.tw.fire_due(t.saturating_mul(1_000));
    for submit in drain(&mut rig.tw) {
        let _ = land(rig, &submit);
    }
}

/// Pump and tick until `done` holds or `secs` of wall time pass.
async fn until(
    rig: &mut Rig,
    rx: &mut Receiver<ArenaOut>,
    lines: &mut Vec<LedgerEvent>,
    secs: u64,
    mut done: impl FnMut(&mut Rig, &[LedgerEvent]) -> bool,
) -> bool {
    let deadline = std::time::Instant::now() + Duration::from_secs(secs);
    let mut beat = 0_u32;
    while std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(25)).await;
        pump(rig, rx, lines);
        beat += 1;
        if beat.is_multiple_of(8) {
            second(rig);
        }
        if done(rig, lines) {
            return true;
        }
    }
    false
}

/// The site's field task, when one runs.
fn task(rig: &Rig) -> Option<tacsim::arena_task::MatchId> {
    rig.tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .and_then(|registry| registry.tasks().live_ids().into_iter().next())
}

/// The last frame `pilot` was sent.
fn frame(rig: &Rig, pilot: ObjId) -> Option<tacsim::arena::step::Frame> {
    rig.tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .and_then(|registry| registry.last_frame(pilot))
        .map(|cockpit| cockpit.frame.clone())
}

/// The hulls the seeded raider stands behind, by the staff attribute.
pub(crate) fn raider_hulls(rig: &Rig) -> Vec<ObjId> {
    let w = rig.tw.world();
    w.module_state::<mush_tactics::module::UnitStore>()
        .map(|store| {
            store
                .units
                .keys()
                .copied()
                .filter(|unit| tacsim::field::raiders::leader_of(w, *unit).is_some())
                .collect()
        })
        .unwrap_or_default()
}

/// **CF.6's raider gate** (doc 32 §6; Q-CF-23, Q-CF-41, Q-CF-42, Q-CF-44).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn cf6_a_seeded_raider_fights_with_nobody_connected_until_it_is_destroyed() {
    let mut rig = stocked_tuned(tune);
    // A prospector works the ground east of the pad; beside it stands an armed hull a hired NPC
    // sits in, no roster and no player behind it.
    let (a, row, a_tvs) = on_pad(&mut rig, 1);
    launched(&mut rig, a, a_tvs[0]);
    to_cell(&mut rig, a_tvs[0], (190, 126));
    let guard = armed(&mut rig, "Ward Dog", (183, 126), 270_000);
    let npc = staff_hand(&mut rig, |ctx| {
        ctx.w
            .create(ObjKind::Npc, "Hired guard", GOD, None)
            .expect("the NPC")
    });
    seat_npc(&mut rig, guard, npc);
    warm(&mut rig);
    // The prospector's carrier leaves the ground first (a staff hand standing in for its
    // departure). With a carrier admitted, the stock doctrine's stand takes the best-ranked
    // hostile and fires only if it has a line to it, never falling back to the next: a carrier
    // with no line from where the raider stands holds every raider gun silent. That is the
    // kernel's doctrine and doc 32 cut CF.3's carrier together, recorded in doc 32 §11 for CF.7.
    staff_hand(&mut rig, |ctx| {
        mush_sim::unplace(ctx, ObjId(row.hull), "test").expect("unplaced");
    });

    // The content's raider, seeded by staff; seeding it again is refused while it stands.
    let listed = run(&mut rig.tw, GOD, "@field/raiders");
    assert!(listed.contains(SKULKS), "{listed}");
    let seeded = run(&mut rig.tw, GOD, &format!("@field/raiders {SKULKS}"));
    assert!(seeded.contains("seeded"), "{seeded}");
    let again = run(&mut rig.tw, GOD, &format!("@field/raiders {SKULKS}"));
    assert!(again.contains("never respawned"), "{again}");
    let raiders = raider_hulls(&rig);
    assert_eq!(
        raiders.len(),
        2,
        "two hulls stand behind the raider's leader"
    );
    let leader = tacsim::field::raiders::leader_of(rig.tw.world(), raiders[0]).expect("a leader");
    for hull in &raiders {
        fragile(&mut rig, *hull);
    }
    let mut rx = egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();
    for _ in 0..3 {
        second(&mut rig);
    }

    // The raider's leader marks the flight to the east Hostile: the ordinary door.
    let (out, queued) = command(&mut rig, leader, "+field/mark E=hostile");
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    land(&mut rig, &queued[0]).expect("the promotion lands");
    // Nobody is connected from here on: the field runs on its own (Q-CF-23).
    for who in [a, npc, leader] {
        rig.tw.world_mut().set_connected(who, false);
    }
    assert!(
        until(&mut rig, &mut rx, &mut lines, 15, |rig, _| task(rig)
            .is_some())
        .await,
        "the field's task opens"
    );
    // The raider is one party, led by its leader; the NPC's hull is the mind's, its seat kept.
    let live = tacsim::field::live_on(rig.tw.world(), &cid(AHUNA)).expect("the live record");
    let raider_party = live
        .parties
        .iter()
        .find(|party| party.key == PartyKey::Fixture { leader })
        .expect("the raider is one fixture party");
    for hull in &raiders {
        assert_eq!(live.party_of(*hull), Some(raider_party.party));
    }
    assert_ne!(live.party_of(guard), Some(raider_party.party));
    // The guard's own frame reaches the host (its seat), and every seat opened at HOLD.
    assert!(
        until(&mut rig, &mut rx, &mut lines, 20, |rig, _| frame(rig, npc)
            .is_some())
        .await,
        "the NPC's own frame reaches the host"
    );
    assert!(
        frame(&rig, npc).is_some_and(|frame| !frame.weapons_free),
        "an IC field opens every seat at weapons hold (Q-CF-41)"
    );

    // The fight runs to the raider's destruction and the field demotes, with nobody connected.
    let fought = until(&mut rig, &mut rx, &mut lines, 120, |rig, _| {
        task(rig).is_none() && site_record(rig).is_some_and(|record| record.open_field().is_none())
    })
    .await;
    let fired = |who: ObjId| {
        lines
            .iter()
            .filter(|line| matches!(line.kind, EventKind::Fired { unit, .. } if unit == who))
            .count()
    };
    let kinds = lines.iter().fold(
        std::collections::BTreeMap::<String, usize>::new(),
        |mut seen, line| {
            let debug = format!("{:?}", line.kind);
            let word = debug
                .split(|c: char| !c.is_ascii_alphanumeric())
                .next()
                .unwrap_or_default()
                .to_owned();
            *seen.entry(word).or_default() += 1;
            seen
        },
    );
    assert!(
        fought,
        "the field demotes on its own; raider shots {:?}, guard shots {}; ledger {kinds:?};          parties {:?}; guard sees {:?}",
        raiders.iter().map(|hull| fired(*hull)).collect::<Vec<_>>(),
        fired(guard),
        live.parties,
        frame(&rig, npc).map(|frame| frame
            .contacts
            .iter()
            .map(|c| (c.designator.to_string(), c.relation, c.range_cm, c.cell))
            .collect::<Vec<_>>())
    );
    assert!(
        raiders.iter().any(|hull| fired(*hull) > 0),
        "the raider fought under its leader's standing marks"
    );
    assert!(
        fired(guard) > 0,
        "the NPC's hull flew the doctrine and answered fire once fired on"
    );
    let record = site_record(&mut rig).expect("the record");
    for hull in &raiders {
        let left = record
            .units
            .iter()
            .find(|left| left.unit == *hull)
            .expect("each raider hull is on the site record");
        assert_eq!(left.condition, LeftCondition::Wreck, "#{} fell", hull.0);
    }
    assert!(
        lines.iter().any(|line| matches!(
            line.kind,
            EventKind::MatchEnded {
                reason: tacsim::arena::state::EndReason::Disengaged
            }
        )),
        "the black box ends on the disengagement"
    );
}

//! The helpers of `field_raiders` (crates/tacsim/tests/domain_field/field_raiders.rs), hoisted
//! here on 2026-11-03 so that the modules which import them can live in any
//! shard binary: a test module can only import from a module of its own
//! binary, and with the helpers inside the test file every importer was
//! pinned to the file's shard (the field cluster was 62 summed test-minutes
//! in one shard). Every item and import is `pub(crate)`; the test half keeps
//! its name and takes them back with `use crate::field_raiders_rig::*`. Each domain
//! root declares this module by path, like support/vertex.rs.
#![allow(dead_code, unused_imports)]

pub(crate) use std::time::Duration;

pub(crate) use mush_core::{GOD, ObjId, ObjKind};
pub(crate) use tacsim::TacsimConfig;
pub(crate) use tacsim::arena::ledger::{EventKind, LedgerEvent};
pub(crate) use tacsim::arena_task::ArenaOut;
pub(crate) use tacsim::field::PartyKey;
pub(crate) use tacsim::surface::site_record::{LeftCondition, SiteKey, SiteRecord};
pub(crate) use tokio::sync::mpsc::Receiver;

pub(crate) use crate::driving_rig::{
    AHUNA, Rig, act_go, ahuna_domain, cid, command, drain, finish, handles, land, launched, on_pad,
    on_pad_at, run, staff_hand, stocked_tuned, to_cell, to_cell_on, typed, warm,
};

/// The raider the content ships.
pub(crate) const SKULKS: &str = "tacsim.raider.ahuna_skulks";

/// A fast field: the lab tempo, a short quiet interval before demotion.
pub(crate) fn tune(cfg: &mut TacsimConfig) {
    cfg.arena.tempo.quiet_ms = 100;
    cfg.arena.tempo.alert_ms = 100;
    cfg.arena.tempo.engaged_ms = 100;
    cfg.arena.snapshot_ms = 500;
    cfg.field.disengage_s = 10;
}

/// Ahuna Flank's record.
pub(crate) fn site_record(rig: &mut Rig) -> Option<SiteRecord> {
    let h = handles(&rig.tw);
    typed(&mut rig.tw, &h.families.sites, &SiteKey(cid(AHUNA)))
}

/// **An armed hull** (the line tank) on Ahuna Flank's ground at `cell`.
pub(crate) fn armed(rig: &mut Rig, name: &str, cell: (i64, i64), heading: u32) -> ObjId {
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
pub(crate) fn fragile(rig: &mut Rig, unit: ObjId) {
    worn(rig, unit, true);
}

/// **Strip `unit`'s armour** (doc 32 cut CF.6b): every facet at its floor, its internals whole —
/// a line tank at 224 per mille, under the doctrine's withdraw edge and over its stand floor, so
/// it goes home and fights from a stand without falling to the first hits.
pub(crate) fn stripped(rig: &mut Rig, unit: ObjId) {
    worn(rig, unit, false);
}

/// Armour at its floor and, with `internals`, internals one above.
pub(crate) fn worn(rig: &mut Rig, unit: ObjId, internals: bool) {
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
                    tacsim::definitions::unit::INTERNALS_CELL if internals => {
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
pub(crate) fn egress(rig: &mut Rig) -> Receiver<ArenaOut> {
    rig.tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .map(|registry| registry.tasks().clone())
        .and_then(|tasks| tasks.take_egress())
        .expect("the registry's egress")
}

/// The world loop's `Wake::ArenaEgress` arm, by hand, keeping every ledger line.
pub(crate) fn pump(rig: &mut Rig, rx: &mut Receiver<ArenaOut>, lines: &mut Vec<LedgerEvent>) {
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
pub(crate) fn second(rig: &mut Rig) {
    let t = rig.tw.world().now().saturating_add(1);
    rig.tw.set_now(t);
    let _ = rig.tw.fire_due(t.saturating_mul(1_000));
    for submit in drain(&mut rig.tw) {
        let _ = land(rig, &submit);
    }
}

/// Pump and tick until `done` holds or `secs` of wall time pass.
pub(crate) async fn until(
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
pub(crate) fn task(rig: &Rig) -> Option<tacsim::arena_task::MatchId> {
    rig.tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .and_then(|registry| registry.tasks().live_ids().into_iter().next())
}

/// The last frame `pilot` was sent.
pub(crate) fn frame(rig: &Rig, pilot: ObjId) -> Option<tacsim::arena::step::Frame> {
    rig.tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .and_then(|registry| registry.last_frame(pilot))
        .map(|cockpit| cockpit.frame.clone())
}

/// The armed hulls the seeded raider stands behind, by the staff attribute: every one that
/// launched from a carrier (doc 32 cut CF.6b), so never the carrier itself.
pub(crate) fn raider_hulls(rig: &Rig) -> Vec<ObjId> {
    let w = rig.tw.world();
    w.module_state::<mush_tactics::module::UnitStore>()
        .map(|store| {
            store
                .units
                .keys()
                .copied()
                .filter(|unit| tacsim::field::raiders::leader_of(w, *unit).is_some())
                .filter(|unit| tacsim::field::raiders::carrier_of(w, *unit).is_some())
                .collect()
        })
        .unwrap_or_default()
}

/// **Seed the content's raider and launch it** (doc 32 cut CF.6b): its carrier on the ground,
/// its hulls aboard until the launch puts them beside the ramp; the hulls and the carrier.
pub(crate) fn seed_and_launch(rig: &mut Rig) -> (Vec<ObjId>, ObjId) {
    let seeded = run(&mut rig.tw, GOD, &format!("@field/raiders {SKULKS}"));
    assert!(seeded.contains("seeded"), "{seeded}");
    let raiders = raider_hulls(rig);
    let carrier =
        tacsim::field::raiders::carrier_of(rig.tw.world(), raiders[0]).expect("a carrier");
    assert!(
        mush_sim::pose_of(rig.tw.world(), carrier).is_some()
            && raiders
                .iter()
                .all(|hull| mush_sim::pose_of(rig.tw.world(), *hull).is_none()),
        "the carrier is on the ground and its hulls aboard"
    );
    let launched = run(&mut rig.tw, GOD, &format!("@field/raiders/launch {SKULKS}"));
    assert!(launched.contains("launch from"), "{launched}");
    let ramp = tacsim::field::raiders::ramp_cell(rig.tw.world(), carrier).expect("a ramp");
    for hull in &raiders {
        let (_, pose) = mush_sim::pose_of(rig.tw.world(), *hull).expect("launched");
        let cell = (pose.x_cm.div_euclid(1_000), pose.y_cm.div_euclid(1_000));
        assert!(
            (cell.0 - ramp[0]).abs().max((cell.1 - ramp[1]).abs()) <= 4,
            "#{} stands beside the ramp: {cell:?} by {ramp:?}",
            hull.0
        );
    }
    let again = run(&mut rig.tw, GOD, &format!("@field/raiders/launch {SKULKS}"));
    assert!(again.contains("nobody aboard"), "{again}");
    (raiders, carrier)
}

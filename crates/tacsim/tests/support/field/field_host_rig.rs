//! The helpers of `field_host` (crates/tacsim/tests/domain_field/field_host.rs), hoisted
//! here on 2026-11-03 so that the modules which import them can live in any
//! shard binary: a test module can only import from a module of its own
//! binary, and with the helpers inside the test file every importer was
//! pinned to the file's shard (the field cluster was 62 summed test-minutes
//! in one shard). Every item and import is `pub(crate)`; the test half keeps
//! its name and takes them back with `use crate::field_host_rig::*`. Each domain
//! root declares this module by path, like support/vertex.rs.
#![allow(dead_code, unused_imports)]

pub(crate) use std::time::Duration;

pub(crate) use mush_core::{GOD, ObjId, ObjKind};
pub(crate) use tacsim::TacsimConfig;
pub(crate) use tacsim::arena::ledger::{EventKind, LedgerEvent};
pub(crate) use tacsim::arena::party::Relation;
pub(crate) use tacsim::arena_task::ArenaOut;
pub(crate) use tacsim::surface::site_record::{LeftCondition, PromotedBy, SiteKey, SiteRecord};
pub(crate) use tokio::sync::mpsc::Receiver;

pub(crate) use crate::driving_rig::{
    AHUNA, Rig, cid, command, drain, handles, land, land_composed, launched, on_pad, on_pad_as,
    reboot, run, site_domain, staff_hand, stocked_tuned, to_cell, typed, warm,
};

/// A fast field: the simulator's lab tempo, a snapshot every half second, a short quiet
/// interval before demotion.
pub(crate) fn tune(cfg: &mut TacsimConfig) {
    cfg.arena.tempo.quiet_ms = 100;
    cfg.arena.tempo.alert_ms = 100;
    cfg.arena.tempo.engaged_ms = 100;
    cfg.arena.snapshot_ms = 500;
    cfg.field.disengage_s = 15;
}

/// The armed hull the fixture fields: the first of the shipped armed designs the lane binds —
/// the line tank first, tracked like the prospector, so it stands wherever a rig can drive.
pub(crate) const ARMED: [&str; 3] = [
    "tacsim.unit.line_tank",
    "tacsim.unit.scout_mk1",
    "tacsim.unit.biped_mk1",
];

/// Ahuna Flank's record, as the family holds it.
pub(crate) fn site_record(rig: &mut Rig) -> Option<SiteRecord> {
    site_record_at(rig, AHUNA)
}

/// A site's record by its port entry (doc 32 cut S.2).
pub(crate) fn site_record_at(rig: &mut Rig, entry: &str) -> Option<SiteRecord> {
    let h = handles(&rig.tw);
    typed(&mut rig.tw, &h.families.sites, &SiteKey(cid(entry)))
}

/// **The fixture's armed hull**: a Thing with the first armed unit spec the lane binds, placed
/// on Ahuna Flank's ground at `cell` facing `heading`.
pub(crate) fn armed(rig: &mut Rig, name: &str, cell: (i64, i64), heading: u32) -> ObjId {
    armed_on(rig, AHUNA, name, cell, heading)
}

/// [`armed`] on any site's ground, by its port entry (doc 32 cut S.2).
pub(crate) fn armed_on(
    rig: &mut Rig,
    entry: &str,
    name: &str,
    cell: (i64, i64),
    heading: u32,
) -> ObjId {
    let domain = site_domain(&rig.tw, entry);
    staff_hand(rig, |ctx| {
        let spec = {
            let store = ctx
                .w
                .module_state::<tacsim::TacsimStore>()
                .expect("the layer");
            let bindings = &store.definitions().bindings;
            ARMED
                .iter()
                .find_map(|id| bindings.get(&cid(id)).map(|binding| binding.spec.clone()))
                .unwrap_or_else(|| {
                    panic!(
                        "no armed design is bound; bound: {:?}",
                        bindings.keys().collect::<Vec<_>>()
                    )
                })
        };
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

/// **The fixture's seat**: `pilot` in `unit`'s cockpit station, every cockpit channel granted,
/// its console the room the pilot stands in aboard the carrier — a remote cab, since the armed
/// lease and its boarding are CF.6's.
pub(crate) fn seat(rig: &mut Rig, unit: ObjId, pilot: ObjId) {
    staff_hand(rig, |ctx| {
        ctx.actor = pilot;
        let console = ctx.w.get(pilot).and_then(|object| object.location);
        let station = mush_sim::StationId::new("cockpit").expect("a station id");
        let channels = tacsim::match_::cockpit::cockpit_channels();
        mush_sim::declare_station(
            ctx,
            unit,
            &station,
            tacsim::match_::cockpit::STATION_ROLE,
            &channels,
            console,
            tacsim::match_::cockpit::STATION_PRIORITY,
        )
        .expect("the station");
        mush_sim::request_seat_activation(
            ctx,
            &mush_sim::SeatRequest::Activate {
                unit,
                station,
                npc: pilot,
                granted: channels,
                fallback: None,
            },
        )
        .expect("the seat");
        mush_sim::links::refresh_actor_links(ctx.w, pilot);
    });
}

/// **Make `unit` fragile**: every armour plate at its floor and every section's internals one
/// above it, so the next hits that land wreck it (a fixture, so the gate fights in seconds).
pub(crate) fn fragile(rig: &mut Rig, unit: ObjId) {
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
            Some(smol_str::SmolStr::new_static("test:cf2-fragile")),
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

/// **The world loop's `Wake::ArenaEgress` arm, by hand**, keeping every ledger line the field
/// wrote; answers how many messages it staged.
pub(crate) fn pump(
    rig: &mut Rig,
    rx: &mut Receiver<ArenaOut>,
    lines: &mut Vec<LedgerEvent>,
) -> usize {
    let Some(tasks) = rig
        .tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .map(|registry| registry.tasks().clone())
    else {
        return 0;
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
    staged
}

/// One world second: the ticks due, and every tacsim submission they queued, landed.
pub(crate) fn second(rig: &mut Rig) -> Vec<String> {
    second_with(rig, false)
}

/// [`second`] with the ticks' submissions landed on the composed path (doc 36a lane 4;
/// `driving::land_composed`, production's path): the host's close act then writes the hull's
/// black box close line, which the plain prepare never does.
pub(crate) fn second_composed(rig: &mut Rig) -> Vec<String> {
    second_with(rig, true)
}

pub(crate) fn second_with(rig: &mut Rig, composed: bool) -> Vec<String> {
    let t = rig.tw.world().now().saturating_add(1);
    rig.tw.set_now(t);
    let _ = rig.tw.fire_due(t.saturating_mul(1_000));
    let queued = drain(&mut rig.tw);
    queued
        .iter()
        .filter_map(|submit| {
            if composed {
                land_composed(rig, submit).err()
            } else {
                land(rig, submit).err()
            }
        })
        .collect()
}

/// Pump and tick until `done` holds or `secs` of wall time pass; answers whether it held.
pub(crate) async fn until(
    rig: &mut Rig,
    rx: &mut Receiver<ArenaOut>,
    lines: &mut Vec<LedgerEvent>,
    secs: u64,
    done: impl FnMut(&mut Rig, &[LedgerEvent]) -> bool,
) -> bool {
    until_with(rig, rx, lines, secs, false, done).await
}

/// [`until`] with every tick's submissions landed on the composed path ([`second_composed`]).
pub(crate) async fn until_composed(
    rig: &mut Rig,
    rx: &mut Receiver<ArenaOut>,
    lines: &mut Vec<LedgerEvent>,
    secs: u64,
    done: impl FnMut(&mut Rig, &[LedgerEvent]) -> bool,
) -> bool {
    until_with(rig, rx, lines, secs, true, done).await
}

pub(crate) async fn until_with(
    rig: &mut Rig,
    rx: &mut Receiver<ArenaOut>,
    lines: &mut Vec<LedgerEvent>,
    secs: u64,
    composed: bool,
    mut done: impl FnMut(&mut Rig, &[LedgerEvent]) -> bool,
) -> bool {
    let deadline = std::time::Instant::now() + Duration::from_secs(secs);
    let mut beat = 0_u32;
    while std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(25)).await;
        pump(rig, rx, lines);
        beat += 1;
        if beat.is_multiple_of(8) {
            let _ = second_with(rig, composed);
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

/// The last cockpit frame `pilot` was sent.
pub(crate) fn frame(rig: &Rig, pilot: ObjId) -> Option<tacsim::arena::step::Frame> {
    rig.tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .and_then(|registry| registry.last_frame(pilot))
        .map(|cockpit| cockpit.frame.clone())
}

/// `unit`'s first weapon's component id.
pub(crate) fn weapon_of(rig: &Rig, unit: ObjId) -> String {
    let store = rig
        .tw
        .world()
        .module_state::<tacsim::TacsimStore>()
        .expect("the layer");
    let cells = mush_tactics::module::unit_of(rig.tw.world(), unit).expect("the unit");
    tacsim::match_::weapon_summaries(store.definitions(), cells)
        .first()
        .map(|weapon| weapon.id.to_string())
        .expect("an armed hull")
}

/// Hoisted from `field_gc3` (doc 36a lane 3): `field_gc4` and `field_director` share it.
/// **Write `condition` on `section` of `unit`, and its internals to `internals`** (or leave them
/// when `None`): the staff fixture standing in for damage.
pub(crate) fn set_section(
    rig: &mut crate::driving_rig::Rig,
    unit: ObjId,
    section: &str,
    condition: Option<&str>,
    internals: Option<i64>,
) {
    staff_hand(rig, |ctx| {
        let cells = mush_tactics::module::unit_of(ctx.w, unit)
            .expect("the unit")
            .clone();
        let id = mush_tactics::SectionId::new(section).expect("a section id");
        assert!(cells.section(&id).is_some(), "#{} has a {section}", unit.0);
        let mut ops = Vec::new();
        if let Some(condition) = condition {
            ops.push(mush_tactics::UnitMutationOp::SetSectionCondition {
                section: id.clone(),
                condition: mush_tactics::ConditionId::new(condition).expect("a condition"),
            });
        }
        if let Some(value) = internals {
            ops.push(mush_tactics::UnitMutationOp::SetCell {
                addr: mush_tactics::CellAddr::Section {
                    section: id,
                    cell: mush_tactics::CellId::new(tacsim::definitions::unit::INTERNALS_CELL)
                        .expect("a cell id"),
                },
                value,
            });
        }
        let plan = mush_tactics::UnitMutationPlan::new(
            ops,
            None,
            Some(smol_str::SmolStr::new_static("test:gc3-damage")),
        )
        .expect("a plan");
        mush_tactics::module::apply_plan(ctx, unit, &plan).expect("applied");
    });
}

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
/// interval before demotion ([`quiet_s`]).
pub(crate) fn tune(cfg: &mut TacsimConfig) {
    cfg.arena.tempo.quiet_ms = 100;
    cfg.arena.tempo.alert_ms = 100;
    cfg.arena.tempo.engaged_ms = 100;
    cfg.arena.snapshot_ms = 500;
    cfg.field.disengage_s = quiet_s();
}

/// **The rigs' short quiet interval**, seconds (doc 49 unit CC.2): five seconds past the
/// slowest cycle of the armed hulls the field rigs field (the line tank's mounts, and CF.6c's
/// `tv_escort`'s), read from the shipped content. A field disengages after `disengage_s` with no
/// hostile fire, so an interval at or under a gun's cycle closes the field between two of its
/// rounds. 20 under the ruled palette's 15 s cannon; the rigs held 15 against the 6 s cannon
/// until CC.2.
pub(crate) fn quiet_s() -> u32 {
    use tacsim::definitions::unit::ComponentRole;
    let catalog = tacsim::builtin_definitions().expect("the shipped content binds");
    let slowest = ["tacsim.unit.line_tank", "tacsim.unit.tv_escort"]
        .into_iter()
        .flat_map(|unit| {
            let spec = catalog
                .specs
                .get(&mush_core::CanonicalId::new(unit).expect("an id"))
                .unwrap_or_else(|| panic!("{unit} is fielded"));
            spec.components
                .iter()
                .filter_map(|slot| match &slot.role {
                    ComponentRole::Weapon { weapon, .. } => {
                        catalog.weapons.get(weapon).map(|def| def.cycle_ms)
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        })
        .max()
        .expect("an armed hull");
    slowest.div_ceil(1_000) + 5
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

/// **`operator` leaves their rig in bay slot `slot` on `HOLD`**, by their own command from
/// aboard its carrier (doc 45 units AP.4 and AP.7). A rig nobody sits in is its mind's in a
/// field and goes home on `RETURN`, the posture every rig holds until its player sets
/// another. A story whose rig is to stand where it was left says so, as its player would.
pub(crate) fn left_on_hold(rig: &mut Rig, operator: ObjId, slot: u8) {
    use tacsim::posture::Posture;
    use tacsim::posture::order::{POSTURE_SET, line};
    assert_eq!(
        run(&mut rig.tw, operator, &format!("+tv/{slot} posture hold")),
        line(POSTURE_SET, &format!("Rig {slot}"), Posture::Hold),
        "rig {slot} is left on HOLD"
    );
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

/// **The field rigs' wait-loop pace** (doc 43 unit TS.10a; doc 43c §1): one 25-ms tokio beat at
/// a time until a deadline `secs` of tokio time away, answering at each beat how many world
/// seconds the loop owes the world.
///
/// - **On a paused clock** (`#[tokio::test(flavor = "current_thread", start_paused = true)]`)
///   the world is owed one second for each second of virtual time the arena has seen, so the
///   world and the arena it hosts keep one clock. The deadline is virtual too: a field that holds
///   for 90 s costs its pulses' CPU, not 90 s of wall.
/// - **On a running clock** (the multi-thread tests and the dispatch stories, whose Registry and
///   minds run in real time) it is the loop as it was: one world second every eighth beat, so the
///   world runs five fake seconds to each real one of the arena's. A test that leans on that ratio
///   pairs two clocks; converting its module to the paused clock is what shows it.
pub(crate) struct Pace {
    deadline: tokio::time::Instant,
    paused: bool,
    beat: u32,
    next_second: tokio::time::Instant,
}

/// One beat of [`Pace`].
pub(crate) const BEAT: Duration = Duration::from_millis(25);

impl Pace {
    /// A pace whose deadline is `secs` of tokio time from now.
    pub(crate) fn new(secs: u64) -> Self {
        let now = tokio::time::Instant::now();
        Self {
            deadline: now + Duration::from_secs(secs),
            paused: clock_is_paused(),
            beat: 0,
            next_second: now + Duration::from_secs(1),
        }
    }

    /// Sleep one beat, unless the deadline has passed (`None`); answers the world seconds owed.
    pub(crate) async fn beat(&mut self) -> Option<u32> {
        if tokio::time::Instant::now() >= self.deadline {
            return None;
        }
        tokio::time::sleep(BEAT).await;
        self.beat = self.beat.wrapping_add(1);
        if !self.paused {
            return Some(u32::from(self.beat.is_multiple_of(8)));
        }
        let now = tokio::time::Instant::now();
        let mut owed = 0;
        while self.next_second <= now {
            owed += 1;
            self.next_second += Duration::from_secs(1);
        }
        Some(owed)
    }
}

/// **Whether tokio's clock is paused** on this runtime: a paused clock does not move while the
/// thread works without yielding (it auto-advances only when the runtime parks), and a running
/// one does. Spins on the std clock for 50 µs; never parks the runtime.
pub(crate) fn clock_is_paused() -> bool {
    let tokio_then = tokio::time::Instant::now();
    let spun = std::time::Instant::now();
    while spun.elapsed() < Duration::from_micros(50) {
        std::hint::spin_loop();
    }
    tokio::time::Instant::now() == tokio_then
}

/// Pump and tick until `done` holds or `secs` of tokio time pass ([`Pace`]); answers whether it
/// held.
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
    let mut pace = Pace::new(secs);
    while let Some(owed) = pace.beat().await {
        pump(rig, rx, lines);
        for _ in 0..owed {
            let _ = second_with(rig, composed);
        }
        if done(rig, lines) {
            return true;
        }
    }
    false
}

/// **The arena's answer to what the world just did**, with no world second passing: pump the
/// egress beat by beat until `done` holds or `secs` of tokio time pass; answers whether it held.
///
/// A story that steps world seconds synchronously (`field_mw7::pass`) gives the arena no turn:
/// on a current-thread runtime the task runs only while the test awaits. On two worker threads
/// the arena ran beside those seconds, so an assertion straight after them read its answer by a
/// race the test happened to win (doc 43 TS.10a). The world's clock stands still here, so a
/// story's count of world seconds is unchanged by the wait.
pub(crate) async fn settle(
    rig: &mut Rig,
    rx: &mut Receiver<ArenaOut>,
    lines: &mut Vec<LedgerEvent>,
    secs: u64,
    mut done: impl FnMut(&mut Rig, &[LedgerEvent]) -> bool,
) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    loop {
        pump(rig, rx, lines);
        if done(rig, lines) {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(BEAT).await;
    }
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

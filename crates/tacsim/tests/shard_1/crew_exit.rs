//! **Doc 32 cut CF.5 — crew and exit**, over the composed world (`driving`'s keyed rig on Ahuna
//! Flank), the arena's egress pumped by hand as `field_host` pumps it.
//!
//! The cut's gate: a unit recovers out of a running field at its carrier's ramp and the field
//! carries on without it; a stranded player is at port after the extraction delay; a departure
//! with a unit out is refused without the leave-behind confirmation and files with it.

use std::time::Duration;

use mush_core::{GOD, ObjId, ObjKind};
use tacsim::TacsimConfig;
use tacsim::arena_task::ArenaOut;
use tacsim::field::{CrewLocation, crew_of};
use tacsim::port::voyage::{LEAVE_HINT, TV_OUT, VoyageState};
use tacsim::surface::excursion::{ActKind, ActResult, RigKey};
use tacsim::surface::site_record::{SiteKey, SiteRecord};
use tokio::sync::mpsc::Receiver;

use crate::driving::{
    AHUNA, Rig, ahuna_domain, cid, command, drain, handles, land, launched, on_pad, on_pad_as, run,
    staff_hand, stocked, stocked_tuned, typed, warm,
};

/// A fast field and a short extraction: the simulator's lab tempo, a quiet interval of fifteen
/// seconds, an extraction of an IC minute.
fn tune(cfg: &mut TacsimConfig) {
    cfg.arena.tempo.quiet_ms = 100;
    cfg.arena.tempo.alert_ms = 100;
    cfg.arena.tempo.engaged_ms = 100;
    cfg.arena.snapshot_ms = 500;
    cfg.field.disengage_s = 15;
    cfg.field.extract_s = 60;
}

/// The armed hull the fixture fields (`field_host`'s list).
const ARMED: [&str; 3] = [
    "tacsim.unit.line_tank",
    "tacsim.unit.scout_mk1",
    "tacsim.unit.biped_mk1",
];

fn site_record(rig: &mut Rig) -> Option<SiteRecord> {
    let h = handles(&rig.tw);
    typed(&mut rig.tw, &h.families.sites, &SiteKey(cid(AHUNA)))
}

/// **An armed hull** on Ahuna Flank's ground at `cell` facing `heading` (`field_host`'s fixture).
fn armed(rig: &mut Rig, name: &str, cell: (i64, i64), heading: u32) -> ObjId {
    let domain = ahuna_domain(&rig.tw);
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
                .expect("an armed design is bound")
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

/// **The seat** (`field_host`'s fixture): `pilot` in `unit`'s cockpit, its console the room the
/// pilot stands in.
fn seat(rig: &mut Rig, unit: ObjId, pilot: ObjId) {
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

/// **Make `unit` fragile** (`field_host`'s fixture): armour at its floor, internals one above.
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
            Some(smol_str::SmolStr::new_static("test:cf5-fragile")),
        )
        .expect("a plan");
        mush_tactics::module::apply_plan(ctx, unit, &plan).expect("applied");
    });
}

fn egress(rig: &mut Rig) -> Receiver<ArenaOut> {
    rig.tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .map(|registry| registry.tasks().clone())
        .and_then(|tasks| tasks.take_egress())
        .expect("the registry's egress")
}

/// The world loop's `Wake::ArenaEgress` arm, by hand.
fn pump(rig: &mut Rig, rx: &mut Receiver<ArenaOut>) {
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

/// What the watched player heard from the ticks, and every refused landing (the gate's diagnostics).
static HEARD: std::sync::Mutex<(Option<ObjId>, String)> =
    std::sync::Mutex::new((None, String::new()));

fn watch(who: ObjId) {
    if let Ok(mut heard) = HEARD.lock() {
        heard.0 = Some(who);
    }
}

fn heard() -> String {
    HEARD
        .lock()
        .map(|heard| heard.1.clone())
        .unwrap_or_default()
}

/// One world second: the ticks due, and every tacsim submission they queued, landed.
fn second(rig: &mut Rig) {
    let t = rig.tw.world().now().saturating_add(1);
    rig.tw.set_now(t);
    let events = rig.tw.fire_due(t.saturating_mul(1_000));
    let mut log = String::new();
    if let Some(who) = HEARD.lock().ok().and_then(|heard| heard.0) {
        for line in rig.tw.seen_by(&events, who) {
            log.push_str(&line);
            log.push('\n');
        }
    }
    for submit in drain(&mut rig.tw) {
        if let Err(why) = land(rig, &submit) {
            log.push_str(&format!("refused {}: {why}\n", submit.action.key().id));
        }
    }
    if let Ok(mut heard) = HEARD.lock() {
        heard.1.push_str(&log);
    }
}

/// Pump and tick until `done` holds or `secs` of wall time pass.
async fn until(
    rig: &mut Rig,
    rx: &mut Receiver<ArenaOut>,
    secs: u64,
    mut done: impl FnMut(&mut Rig) -> bool,
) -> bool {
    let deadline = std::time::Instant::now() + Duration::from_secs(secs);
    let mut beat = 0_u32;
    while std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(25)).await;
        pump(rig, rx);
        beat += 1;
        if beat.is_multiple_of(8) {
            second(rig);
        }
        if done(rig) {
            return true;
        }
    }
    false
}

/// `tv`'s act in flight, as its due instant.
fn act_due(rig: &mut Rig, tv: ObjId) -> Option<i64> {
    let h = handles(&rig.tw);
    typed(&mut rig.tw, &h.families.excursions, &RigKey(tv))
        .and_then(|record| record.act)
        .map(|act| act.due_ic)
}

/// **On to IC instant `target`** in half-minute steps of world time, every tick due fired and every
/// submission landed on the way.
fn to_ic(rig: &mut Rig, target: i64) {
    for _ in 0..2_000 {
        if rig.tw.world().world_ic_now().at >= target {
            break;
        }
        for _ in 0..30 {
            second(rig);
        }
    }
    second(rig);
}

fn task(rig: &Rig) -> Option<tacsim::arena_task::MatchId> {
    rig.tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .and_then(|registry| registry.tasks().live_ids().into_iter().next())
}

fn frame(rig: &Rig, pilot: ObjId) -> Option<tacsim::arena::step::Frame> {
    rig.tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .and_then(|registry| registry.last_frame(pilot))
        .map(|cockpit| cockpit.frame.clone())
}

fn weapon_of(rig: &Rig, unit: ObjId) -> String {
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

fn in_field(rig: &Rig, unit: ObjId) -> bool {
    mush_sim::deployed::mark_of(rig.tw.world(), unit).is_some_and(|mark| mark.field.is_some())
}

/// **CF.5's gate, recovery and extraction** (doc 32 §6; Q-CF-18, Q-CF-36, Q-CF-38, Q-CF-39):
/// A and B fight in armed hulls while C's prospector stands on the ramp. C recovers the rig out
/// of the running field — checked in, named on the record's promotion, then driven into the bay
/// by its own recover — and the field carries on. B fights from inside the hull; when it is
/// wrecked and the field demotes, B is stranded on the site, asks for extraction, and is at a
/// safe port after the delay while the wreck stays where it fell.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn cf5_a_rig_recovers_out_of_a_running_field_and_a_stranded_pilot_is_extracted() {
    let mut rig = stocked_tuned(tune);
    let (a, _, _) = on_pad(&mut rig, 1);
    let (b, _, _) = on_pad_as(&mut rig, "Other", 2, "Tally Moth", 1);
    let (c, _, c_tvs) = on_pad_as(&mut rig, "Third", 3, "Lone Moth", 1);
    let prospector = c_tvs[0];
    launched(&mut rig, c, prospector);
    let hull_a = armed(&mut rig, "Red Hound", (110, 124), 90_000);
    let hull_b = armed(&mut rig, "Grey Hound", (114, 124), 270_000);
    seat(&mut rig, hull_a, a);
    seat(&mut rig, hull_b, b);
    // B flies from inside the hull: a cab, not a remote console.
    staff_hand(&mut rig, |ctx| {
        ctx.w.move_to(b, hull_b).expect("B climbs into the hull");
    });
    fragile(&mut rig, hull_b);
    warm(&mut rig);
    let mut rx = egress(&mut rig);
    for _ in 0..3 {
        second(&mut rig);
    }

    // The field opens: A's fire order at B's hull.
    let (out, queued) = command(&mut rig, a, "+field/fire E");
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    land(&mut rig, &queued[0]).expect("the promotion lands");
    assert!(
        until(&mut rig, &mut rx, 10, |rig| task(rig).is_some()).await,
        "the field's task opens"
    );
    for unit in [hull_a, hull_b, prospector] {
        assert!(in_field(&rig, unit), "#{} is in the field", unit.0);
    }

    // (1) Recovery as exit: the prospector on the ramp leaves the running field.
    let preview = run(&mut rig.tw, c, "+tv/1 recover");
    assert!(preview.contains("is in a field"), "{preview}");
    watch(c);
    let asked = run(&mut rig.tw, c, "+tv/1 recover go");
    assert!(asked.contains("makes for the ramp"), "{asked}");
    let exited = until(&mut rig, &mut rx, 20, |rig| {
        !in_field(rig, prospector)
            && site_record(rig).is_some_and(|record| record.exited(prospector))
    })
    .await;
    let state = if exited {
        String::new()
    } else {
        format!(
            "in field: {}; exits: {:?}; C hears: {}",
            in_field(&rig, prospector),
            site_record(&mut rig).and_then(|record| record.promoted.map(|open| open.exits)),
            run(&mut rig.tw, c, "+tv/1")
        )
    };
    assert!(
        exited,
        "the prospector leaves the field and the record names its exit: {state}"
    );
    // The recover the host submitted as C is in flight: on to its due instant.
    assert!(
        until(&mut rig, &mut rx, 10, |rig| act_due(rig, prospector)
            .is_some())
        .await,
        "the recover starts once the record names the exit: {}",
        heard()
    );
    let due = act_due(&mut rig, prospector).expect("the recover's due instant");
    to_ic(&mut rig, due);
    let home = until(&mut rig, &mut rx, 30, |rig| {
        let h = handles(&rig.tw);
        typed(&mut rig.tw, &h.families.excursions, &RigKey(prospector)).is_some_and(|record| {
            record.excursion.is_none()
                && record.last.as_ref().is_some_and(|last| {
                    last.kind == ActKind::Recover && last.result == ActResult::Done
                })
        })
    })
    .await;
    let record = {
        let h = handles(&rig.tw);
        typed(&mut rig.tw, &h.families.excursions, &RigKey(prospector))
    };
    assert!(
        home,
        "the recover into the bay runs once the rig is out: {record:?}\nheard: {}",
        heard()
    );
    assert!(task(&rig).is_some(), "the field carries on without it");
    for unit in [hull_a, hull_b] {
        assert!(in_field(&rig, unit), "#{} is still in the field", unit.0);
    }
    // The field's own surface refusal still stands for everything else.
    let drive = run(&mut rig.tw, c, "+tv/1 drive 120,116 go");
    assert!(drive.contains("Field 1 is being fought"), "{drive}");

    // (2) A wrecks B's hull; the field demotes; B, inside it, is stranded.
    let gun = weapon_of(&rig, hull_a);
    let near = |cell: Option<(u16, u16)>| {
        cell.is_some_and(|(x, y)| x.abs_diff(114) <= 2 && y.abs_diff(124) <= 2)
    };
    let wrecked = until(&mut rig, &mut rx, 150, |rig| {
        if let Some(contact) = frame(rig, a).and_then(|frame| {
            frame
                .contacts
                .iter()
                .find(|contact| near(contact.cell))
                .cloned()
        }) && frame(rig, a).is_some_and(|frame| frame.lock != Some(contact.id))
        {
            let _ = run(&mut rig.tw, a, &format!("LOCK {}", contact.designator));
        }
        let _ = run(&mut rig.tw, a, &format!("FIRE {gun}"));
        frame(rig, b).is_some_and(|frame| frame.wreck)
    })
    .await;
    assert!(wrecked, "B's hull is wrecked by A's fire");
    assert!(
        until(&mut rig, &mut rx, 40, |rig| {
            task(rig).is_none()
                && site_record(rig).is_some_and(|record| record.open_field().is_none())
        })
        .await,
        "the field demotes after its quiet interval"
    );
    let crew = crew_of(rig.tw.world(), b)
        .cloned()
        .expect("B's crew record");
    assert!(
        matches!(&crew.location, CrewLocation::Stranded { unit, .. } if *unit == hull_b),
        "B is stranded in the wreck: {crew:?}"
    );
    let remote = crew_of(rig.tw.world(), a)
        .cloned()
        .expect("A's crew record");
    assert!(
        matches!(remote.location, CrewLocation::Carrier { .. }),
        "A flew from a console aboard a carrier: {remote:?}"
    );

    // (3) Extraction: B asks; after the delay B is at a safe port, the hull where it fell.
    let refused = run(&mut rig.tw, a, "+sortie/extract");
    assert!(refused.contains("You are not stranded"), "{refused}");
    let asked = run(&mut rig.tw, b, "+sortie/extract");
    assert!(asked.contains("Extraction requested"), "{asked}");
    let again = run(&mut rig.tw, b, "+sortie/extract");
    assert!(again.contains("already on its way"), "{again}");
    let fell = mush_sim::pose_of(rig.tw.world(), hull_b).map(|(_, pose)| (pose.x_cm, pose.y_cm));
    let due = crew_of(rig.tw.world(), b)
        .and_then(|crew| crew.extraction.as_ref().map(|coming| coming.due_ic))
        .expect("the extraction's due instant");
    to_ic(&mut rig, due);
    let extracted = until(&mut rig, &mut rx, 30, |rig| {
        crew_of(rig.tw.world(), b)
            .is_some_and(|crew| matches!(crew.location, CrewLocation::Port { .. }))
    })
    .await;
    assert!(extracted, "B is at port after the extraction delay");
    let CrewLocation::Port { port } = crew_of(rig.tw.world(), b).cloned().expect("B").location
    else {
        unreachable!("checked above");
    };
    let arrival = tacsim::ic_return::PORTS
        .iter()
        .find(|known| known.id == port.as_str())
        .and_then(|known| tacsim::ic_return::arrival_room(rig.tw.world(), known))
        .expect("the port's arrival room");
    assert_eq!(
        rig.tw.world().get(b).and_then(|object| object.location),
        Some(arrival),
        "B stands in {port}'s arrival room"
    );
    assert_eq!(
        mush_sim::pose_of(rig.tw.world(), hull_b).map(|(_, pose)| (pose.x_cm, pose.y_cm)),
        fell,
        "the hull stays where it fell"
    );
}

/// **CF.5's gate, the leave-behind confirmation** (doc 27 Q-H10): with rig 1 out on the ground,
/// `+ship/lift ceres` refuses with the rig's own line and the confirmation it needs; with
/// `leave=1` it files, and the filed voyage records the rig left behind.
#[test]
fn cf5_a_departure_with_a_rig_out_needs_the_leave_behind_confirmation() {
    let mut rig = stocked();
    let (pilot, row, tvs) = on_pad(&mut rig, 1);
    let tv = tvs[0];
    let hull = ObjId(row.hull);
    launched(&mut rig, pilot, tv);
    let out_line = TV_OUT
        .replace("{n}", "1")
        .replace("{site}", "Ahuna Flank")
        .replace("{name}", "Salt Moth");
    let (out, queued) = command(&mut rig, pilot, "+ship/lift ceres");
    assert_eq!(
        out.trim(),
        format!("{out_line}{}", LEAVE_HINT.replace("{list}", "1"))
    );
    assert!(queued.is_empty(), "nothing is filed");
    let (out, queued) = command(&mut rig, pilot, "+ship/lift ceres leave=2");
    assert!(
        out.contains(&out_line),
        "naming another slot is no confirmation: {out}"
    );
    assert!(queued.is_empty());
    let (out, queued) = command(&mut rig, pilot, "+ship/lift ceres leave=1");
    assert_eq!(queued.len(), 1, "the burn is filed: {out}");
    assert!(out.contains("Left behind by your order: #"), "{out}");
    let filed = match rig
        .tw
        .world()
        .module_state::<tacsim::TacsimStore>()
        .expect("the layer")
        .voyage_state(hull)
    {
        VoyageState::Present(voyage) => voyage.pending.clone().expect("a filed passage"),
        VoyageState::Absent | VoyageState::Unreadable => panic!("the voyage is filed"),
    };
    assert_eq!(
        filed.left_behind,
        vec![tv],
        "the decision is on the filed voyage"
    );
}

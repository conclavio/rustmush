//! **The crew-custody rig** (doc 42 cut CR.1; Q-42-25): story (a′) to the load — GC.5's
//! non-friend load rewritten — and its review twin, one script over both grounds of
//! `grounds_rig`, hoisted here from `field_gc5` so the 256 runs stay in shard_1 with GC.5 and the
//! 512 runs and `crew_custody` stand in a shard with room (doc 43 rule 2.3 #3, #14). Every item is
//! `pub(crate)`; each shard root declares this module by path beside `grounds_rig`.
#![allow(dead_code)]

use mush_core::ObjId;
use tacsim::arena::ledger::LedgerEvent;
use tacsim::field::{CrewLocation, crew_of};
use tacsim::surface::salvage::ClaimSeam;
use tacsim::surface::site_record::LeftCondition;

use crate::driving_rig::{
    command, launched_at, on_pad_as_at, ramp_at, run, staff_hand, stocked_tuned, store, to_cell_on,
    warm,
};
use crate::field_host_rig::{armed_on, egress, seat, second, site_record_at, task, tune, until};
use crate::grounds_rig::Ground;

/// Short tow stages and a short extraction, on the field host's lab tempo and its default
/// disengagement (doc 43 TS.16): no body here acts inside a field, it waits for it to settle.
pub(crate) fn quick(cfg: &mut tacsim::TacsimConfig) {
    tune(cfg);
    cfg.field.idle_demote_s = 3_600;
    cfg.field.extract_s = 600;
    cfg.recovery.inspect_s = 30;
    cfg.recovery.attach_s = 30;
    cfg.recovery.load_s = 60;
}

/// **Destroy `unit`'s power component** (`field_gc4::powerless`): no power is no control, a
/// casualty.
pub(crate) fn powerless(rig: &mut crate::driving_rig::Rig, unit: ObjId) {
    staff_hand(rig, |ctx| {
        let plan = mush_tactics::UnitMutationPlan::new(
            vec![mush_tactics::UnitMutationOp::SetComponentCondition {
                component: mush_tactics::unit::ComponentId::new("power").expect("an id"),
                condition: mush_tactics::ConditionId::new("destroyed").expect("a condition"),
            }],
            None,
            Some(smol_str::SmolStr::new_static("test:cr1-powerless")),
        )
        .expect("a plan");
        mush_tactics::module::apply_plan(ctx, unit, &plan).expect("applied");
    });
}

/// Seconds until `done` holds, at most `limit`, every tick's submissions landed; a refused tow
/// landing fails (`field_gc4::seconds_until`).
pub(crate) fn seconds_until(
    rig: &mut crate::driving_rig::Rig,
    limit: u32,
    mut done: impl FnMut(&mut crate::driving_rig::Rig) -> bool,
) -> bool {
    for _ in 0..limit {
        if done(rig) {
            return true;
        }
        let errors: Vec<String> = second(rig)
            .into_iter()
            .filter(|error| error.contains(tacsim::surface::tow::TOW_ACTION))
            .collect();
        assert!(errors.is_empty(), "a tow landing refused: {errors:?}");
    }
    done(rig)
}

/// **The non-friend load, done** ([`non_friend_loaded`]): who and what it left where.
pub(crate) struct Loaded {
    /// The captain, captured in the escort.
    pub(crate) a: ObjId,
    /// The escort, a casualty in the stranger's bay.
    pub(crate) escort: ObjId,
    /// The stranger.
    pub(crate) b: ObjId,
    /// The stranger's carrier, the captor.
    pub(crate) b_carrier: ObjId,
    /// The stranger's rig, which towed and loaded the escort.
    pub(crate) tug: ObjId,
    /// The IC instant before the load.
    pub(crate) before: i64,
    /// The arena's egress and the ledger lines read so far.
    pub(crate) rx: tokio::sync::mpsc::Receiver<tacsim::arena_task::ArenaOut>,
    pub(crate) lines: Vec<LedgerEvent>,
}

/// `entry`'s ramp, as a site cell.
pub(crate) fn ramp_cell_on(rig: &crate::driving_rig::Rig, entry: &str) -> (i64, i64) {
    let (rx, ry, _) = ramp_at(&rig.tw, entry);
    (rx.div_euclid(1_000), ry.div_euclid(1_000))
}

/// Whether `entry`'s field has demoted ([`crate::field_gc4::demoted`] on any ground).
pub(crate) fn demoted_on(rig: &mut crate::driving_rig::Rig, entry: &str) -> bool {
    task(rig).is_none()
        && site_record_at(rig, entry).is_some_and(|record| record.open_field().is_none())
}

/// [`attach`] on any ground.
pub(crate) fn attach_on(
    rig: &mut crate::driving_rig::Rig,
    entry: &str,
    (who, slot): (ObjId, u8),
    tug: ObjId,
    casualty: ObjId,
) {
    let (out, queued) = command(rig, who, &format!("+tv/{slot} tow go"));
    assert_eq!(queued.len(), 1, "one tow start: {out}");
    crate::driving_rig::land(rig, &queued[0]).expect("the tow start lands");
    assert!(
        seconds_until(rig, 120, |rig| site_record_at(rig, entry)
            .and_then(|record| record.tows.towing(tug).map(|op| op.casualty()))
            == Some(casualty)),
        "the tow attaches"
    );
}

/// [`load`] on any ground.
pub(crate) fn load_on(
    rig: &mut crate::driving_rig::Rig,
    entry: &str,
    (who, slot): (ObjId, u8),
    tug: ObjId,
    casualty: ObjId,
) {
    let (out, queued) = command(rig, who, &format!("+tv/{slot} load go"));
    assert_eq!(queued.len(), 1, "one load start: {out}");
    crate::driving_rig::land(rig, &queued[0]).expect("the load start lands");
    assert!(
        seconds_until(rig, 120, |rig| site_record_at(rig, entry).is_some_and(
            |record| {
                record.tows.act_of(tug).is_none() && record.units.iter().all(|l| l.unit != casualty)
            }
        )),
        "the load lands"
    );
}

/// **A bystander's hull of design `id`** on `entry`'s ground at `cell` facing `heading`
/// (`field_crash::hull_of` on any ground).
fn hull_on(
    rig: &mut crate::driving_rig::Rig,
    entry: &str,
    (id, name): (&str, &str),
    cell: (i64, i64),
    heading: u32,
) -> ObjId {
    let domain = crate::driving_rig::site_domain(&rig.tw, entry);
    crate::driving_rig::staff_hand(rig, |ctx| {
        let spec = ctx
            .w
            .module_state::<tacsim::TacsimStore>()
            .expect("the layer")
            .definitions()
            .bindings
            .get(&crate::driving_rig::cid(id))
            .map(|binding| binding.spec.clone())
            .unwrap_or_else(|| panic!("{id} is bound"));
        let obj = ctx
            .w
            .create(mush_core::ObjKind::Thing, name, mush_core::GOD, None)
            .expect("the hull's object");
        mush_tactics::module::create_unit(ctx, obj, &spec).expect("the hull's unit record");
        let pose =
            tacsim::surface::excursion::cell_pose([cell.0, cell.1], heading).expect("a site pose");
        mush_sim::place(ctx, obj, domain, pose).expect("placed on the ground");
        obj
    })
}

/// **The non-friend load** (GC.5's gate, Q-36-16; doc 42 cut CR.1's story (a′) to the load) on
/// `at`'s ground: the captain flies the escort from inside it — seated at its cockpit station and
/// standing in the hull, as a DT flight seats a pilot (doc 42 §3a: `+tv/<n> board` reads a cab as
/// outside the rig until the fix thread's cut lands) — a stranger's mark opens a field, which
/// settles with the escort a casualty and the captain stranded in it; the stranger's rig tows the
/// escort into the stranger's own carrier, and the load completes.
pub(crate) async fn non_friend_loaded(rig: &mut crate::driving_rig::Rig, at: &Ground) -> Loaded {
    let (a, _, tvs) = on_pad_as_at(rig, ("Pilot", 1, "Salt Moth"), at.site(), 1);
    let escort = tvs[0];
    let ramp = ramp_cell_on(rig, at.entry);
    launched_at(rig, at.entry, a, escort);
    to_cell_on(rig, at.entry, escort, (ramp.0 + 1, ramp.1));
    powerless(rig, escort);
    seat(rig, escort, a);
    crate::driving_rig::staff_hand(rig, |ctx| {
        ctx.w.move_to(a, escort).expect("A climbs into the escort");
    });
    // The stranger: a carrier on the same pad with one rig, and an armed hull whose mark opens
    // the field.
    let (b, b_row, b_tvs) = on_pad_as_at(rig, ("Second", 2, "Red Moth"), at.site(), 1);
    let b_carrier = ObjId(b_row.hull);
    let tug = b_tvs[0];
    launched_at(rig, at.entry, b, tug);
    // Parked behind the stranger's hull until the field settles, so the mark east sees only A.
    to_cell_on(rig, at.entry, tug, (ramp.0 - 12, ramp.1));
    let hull = armed_on(rig, at.entry, "Red Hound", (ramp.0 - 8, ramp.1), 90_000);
    seat(rig, hull, b);
    warm(rig);
    let mut rx = egress(rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();
    let (out, queued) = command(rig, b, "+field/mark E=hostile");
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    crate::driving_rig::land(rig, &queued[0]).expect("the promotion lands");
    assert!(
        until(rig, &mut rx, &mut lines, 20, |rig, _| task(rig).is_some()).await,
        "the field opens"
    );
    assert!(
        until(rig, &mut rx, &mut lines, 180, |rig, _| demoted_on(
            rig, at.entry
        ))
        .await,
        "the field settles"
    );
    assert!(
        site_record_at(rig, at.entry).is_some_and(|record| record
            .units
            .iter()
            .any(|left| left.unit == escort && left.condition == LeftCondition::Casualty)),
        "the escort lies a casualty"
    );
    assert!(
        crew_of(rig.tw.world(), a).is_some_and(
            |crew| matches!(&crew.location, CrewLocation::Stranded { unit, .. } if *unit == escort)
        ),
        "A is stranded in the escort: {:?}",
        crew_of(rig.tw.world(), a)
    );

    // The stranger's rig tows the escort and loads it into the Red Moth.
    to_cell_on(rig, at.entry, tug, (ramp.0 + 1, ramp.1 + 1));
    let _ = second(rig);
    attach_on(rig, at.entry, (b, 1), tug, escort);
    let preview = run(&mut rig.tw, b, "+tv/1 load");
    assert!(
        preview.contains("into the Red Moth's bay")
            && preview.contains("custody with a salvage claim owed, never its title")
            && preview.contains("held in your custody aboard the Red Moth"),
        "{preview}"
    );
    let before = rig.tw.world().world_ic_now().at;
    load_on(rig, at.entry, (b, 1), tug, escort);
    assert!(
        tacsim::surface::tow::in_bay_of(rig.tw.world(), escort, b_carrier),
        "the escort is in the stranger's bay"
    );
    Loaded {
        a,
        escort,
        b,
        b_carrier,
        tug,
        before,
        rx,
        lines,
    }
}

/// **GC.5's gate: the non-friend load, rewritten by doc 42 cut CR.1** (Q-36-16, Q-38-22,
/// Q-42-4, Q-42-16; story (a′) to the load) on `at`'s ground. The load completes: the escort is
/// in the stranger's bay, in the stranger's custody with a claim owed (never title), its epoch
/// closed naming the stranger's rig. The captain is captured: out of the escort's seat, standing
/// in the Red Moth's bay room, `Captured { carrier: the Red Moth, captor: the stranger }` from the
/// load's instant, told so; `+sortie/extract` refuses with the held line, `+sortie/condition`
/// says where they are held, and the stranger's `+ship/captives` names them.
pub(crate) async fn non_friend_load_captures_the_crew(at: &Ground) {
    let mut rig = stocked_tuned(quick);
    let Loaded {
        a,
        escort,
        b,
        b_carrier,
        tug,
        before,
        rx: _rx,
        ..
    } = non_friend_loaded(&mut rig, at).await;
    let record = site_record_at(&mut rig, at.entry).expect("the site record");
    let custody = record
        .custody
        .iter()
        .find(|row| row.hull == escort)
        .cloned()
        .expect("the escort is in custody");
    assert_eq!(custody.holder, b, "the carrier's operator holds it");
    assert_eq!(custody.carrier, b_carrier);
    assert_eq!(custody.recovered_by, tug);
    assert_eq!(custody.claim, ClaimSeam::Owed, "a claim owed for GC.6");
    assert!(custody.left_ic.is_some(), "the ground's clock rides along");
    let escort_record = crate::driving_rig::record_of(&mut rig.tw, escort).expect("its rig record");
    assert!(escort_record.sortie.is_none(), "its epoch is closed");
    assert!(
        store(&rig.tw).tv_lease_on(escort).is_some(),
        "custody is not title: the lease still stands"
    );

    // Doc 42 cut CR.1 (Q-42-1, Q-42-4): A is held aboard the Red Moth by its operator.
    let crew = crew_of(rig.tw.world(), a).cloned().expect("A's record");
    match crew.location {
        CrewLocation::Captured {
            carrier,
            captor,
            since_ic,
        } => {
            assert_eq!((carrier, captor), (b_carrier, b), "{crew:?}");
            assert!(since_ic >= before, "{crew:?}");
        }
        other => panic!("A is not captured: {other:?}"),
    }
    assert!(
        crew.extraction.is_none(),
        "no extraction while held: {crew:?}"
    );
    let room = rig
        .tw
        .world()
        .get(a)
        .and_then(|object| object.location)
        .expect("A stands somewhere");
    assert_eq!(
        tacsim::port::ship::hull_of_room(rig.tw.world(), room).map(|(view, _)| view.hull()),
        Some(b_carrier),
        "A stands in the Red Moth's rooms"
    );
    assert!(
        rig.tw
            .world()
            .module_state::<mush_sim::SpatialState>()
            .and_then(|state| state.assignments.get(&escort))
            .is_none_or(|host| host.seats.values().all(|seat| seat.actor != a)),
        "A has left the escort's seat"
    );
    let extract = run(&mut rig.tw, a, "+sortie/extract");
    assert!(
        extract.contains("You are held aboard the Red Moth."),
        "{extract}"
    );
    let condition = run(&mut rig.tw, a, "+sortie/condition");
    assert!(
        condition.contains("held aboard the Red Moth"),
        "{condition}"
    );
    let leave = run(&mut rig.tw, a, "+tv/1 leave");
    assert!(
        leave.contains("You are held aboard the Red Moth."),
        "{leave}"
    );
    let captives = run(&mut rig.tw, b, "+ship/captives");
    assert!(
        captives.contains(&format!("(#{}): held aboard the Red Moth", a.0)),
        "{captives}"
    );
    assert!(
        tacsim::surface::tow::in_bay_of(rig.tw.world(), escort, b_carrier),
        "the hull stays with the carrier"
    );
}

/// **Doc 36a §11 row 4, rewritten by doc 42 cut CR.1: the captor's field close keeps the captive**
/// (Q-42-3) on `at`'s ground. After the non-friend load A is held in the Red Moth's bay room,
/// physically aboard the Red Moth and so among the souls aboard it. The stranger's mark at a
/// bystander's hull opens a second field, and the Red Moth is admitted with A among the souls
/// aboard its row. Its close checks the Red Moth's souls in, and once checked A in as the
/// captor's crew; a record in custody is left alone now: A is still held by the stranger from
/// the same instant, never re-crewed, and the hull stays with the carrier.
pub(crate) async fn captors_field_close_keeps_the_captive(at: &Ground) {
    let mut rig = stocked_tuned(quick);
    let Loaded {
        a,
        escort,
        b,
        b_carrier,
        mut rx,
        mut lines,
        ..
    } = non_friend_loaded(&mut rig, at).await;
    let ramp = ramp_cell_on(&rig, at.entry);
    let held = crew_of(rig.tw.world(), a)
        .map(|crew| crew.location.clone())
        .expect("A's record");
    assert!(
        matches!(held, CrewLocation::Captured { carrier, captor, .. } if carrier == b_carrier && captor == b),
        "{held:?}"
    );

    // A bystander's unarmed rig three cells east of the stranger's hull: the mark's target,
    // nobody's party, the only flight east of it, and nothing in the field fires.
    let stray = hull_on(
        &mut rig,
        at.entry,
        ("tacsim.unit.tv_prospector", "Stray Moth"),
        (ramp.0 - 5, ramp.1),
        90_000,
    );
    let (out, queued) = command(&mut rig, b, "+field/mark E=hostile");
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    crate::driving_rig::land(&mut rig, &queued[0]).expect("the promotion lands");
    assert!(
        until(&mut rig, &mut rx, &mut lines, 20, |rig, _| task(rig)
            .is_some())
        .await,
        "the second field opens"
    );
    let live = tacsim::field::live_on(rig.tw.world(), &crate::driving_rig::cid(at.entry))
        .expect("the live record");
    assert!(
        live.units
            .iter()
            .any(|row| row.unit == b_carrier && row.aboard.contains(&a)),
        "A is among the souls aboard the captor's row: {:?}",
        live.units
    );
    assert!(
        live.units.iter().any(|row| row.unit == stray),
        "the bystander is in the field: {:?}",
        live.units
    );
    assert_eq!(
        tacsim::field::crew_location(rig.tw.world(), a),
        Some(held.clone()),
        "custody wins over the carrier's row"
    );
    assert!(
        until(&mut rig, &mut rx, &mut lines, 180, |rig, _| demoted_on(
            rig, at.entry
        ))
        .await,
        "the second field settles"
    );

    // The captor's check-in left the captive's record alone.
    let crew = crew_of(rig.tw.world(), a).cloned().expect("A's record");
    assert_eq!(crew.location, held, "A is still held: {crew:?}");
    assert!(
        tacsim::surface::tow::in_bay_of(rig.tw.world(), escort, b_carrier),
        "the hull stays with the carrier"
    );
}

//! **The crew-custody rig** (doc 42 cut CR.1; Q-42-25): story (a′) to the load — GC.5's
//! non-friend load rewritten — and its review twin, one script over both grounds of
//! `grounds_rig`, hoisted here from `field_gc5` so the 256 runs stay with GC.5 and the 512 runs
//! with `crew_custody` (doc 43 rule 2.3 #3). Every item is
//! `pub(crate)`; each domain root declares this module by path beside `grounds_rig`.
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

/// **The captive's own credit balance**, minor units.
fn balance_minor(rig: &crate::driving_rig::Rig, who: ObjId) -> i64 {
    tacsim::port::credit::own_account(rig.tw.world(), who).map_or(0, |a| a.posted_balance.minor())
}

/// **The captive's available credit** (posted less held: a lease deposit is held), minor units.
fn available_minor(rig: &crate::driving_rig::Rig, who: ObjId) -> i64 {
    tacsim::port::credit::own_account(rig.tw.world(), who)
        .map_or(0, |a| a.posted_balance.minor() - a.held_amount.minor())
}

/// **Doc 42 cut CR.2's story (a′) whole on `at`'s ground** (Q-42-6, Q-42-7, Q-42-19): after
/// GC.5's load the stranger lifts for Ceres, leaving its rig on the site, and docks. At the docking
/// the captain is handed to the Freeport Registry: set down in Ceres's medical room (the arrivals
/// hall), `Processing { ceres, release_due_ic, charge }` with the release fee's obligation named,
/// told; leaving the port is refused with the processing line, `+sortie/condition` says so, and the
/// captor's `+ship/captives` is empty. With `paid` the captain's account covers the fee and the
/// obligation settles; without, a staff hand empties the account before the docking and the
/// obligation stands unpaid. Either way, after the processing hour the captain is released at the
/// port: `Port { ceres }`, stamped Ceres as the port they know.
pub(crate) async fn handover_at_ceres(at: &Ground, paid: bool) {
    let mut rig = stocked_tuned(quick);
    let Loaded {
        a, b, b_carrier, ..
    } = non_friend_loaded(&mut rig, at).await;
    let before = balance_minor(&rig, a);
    assert!(before > 0, "the captain has credit");
    if !paid {
        let from = tacsim::port::credit::own_account(rig.tw.world(), a)
            .map(|a| a.id.0)
            .expect("A's account");
        let to = tacsim::port::credit::own_account(rig.tw.world(), b)
            .map(|a| a.id.0)
            .expect("B's account");
        let spend = available_minor(&rig, a);
        let paid_out = run(
            &mut rig.tw,
            mush_core::GOD,
            // `@econ/pay` takes minor units, as `field_story`'s 7,000 cr is `700000`.
            &format!("@econ/pay #{from}=#{to} {spend}"),
        );
        assert!(
            available_minor(&rig, a) < 150_000,
            "A cannot cover the fee: {paid_out}"
        );
    }

    // The stranger lifts for Ceres with its rig left on the ground, and docks there.
    let ticket =
        crate::driving_rig::file_and_launch(&mut rig, b, b_carrier, "+ship/lift ceres leave=1");
    let _ = crate::driving_rig::to_ic(&mut rig, ticket.plan.depart_ic + ticket.plan.total_s + 60);
    assert!(
        seconds_until(&mut rig, 120, |rig| matches!(
            crew_of(rig.tw.world(), a).map(|c| &c.location),
            Some(CrewLocation::Processing { .. })
        )),
        "A is handed over at Ceres: {:?}, the carrier {:?}",
        crew_of(rig.tw.world(), a),
        tacsim::port::ship::berth_of(rig.tw.world(), b_carrier)
    );
    let crew = crew_of(rig.tw.world(), a).cloned().expect("A's record");
    let CrewLocation::Processing {
        port,
        release_due_ic,
        charge,
    } = crew.location.clone()
    else {
        unreachable!("matched above");
    };
    assert_eq!(port.as_str(), "ceres");
    let charge = charge.expect("the release fee's obligation is named");
    let room = rig
        .tw
        .world()
        .get(a)
        .and_then(|object| object.location)
        .expect("A stands somewhere");
    assert_eq!(
        tacsim::ic_return::port_of_room(rig.tw.world(), room).map(|p| p.id),
        Some("ceres"),
        "A stands in a room of Ceres"
    );
    let condition = run(&mut rig.tw, a, "+sortie/condition");
    assert!(condition.contains("in processing at Ceres"), "{condition}");
    let captives = run(&mut rig.tw, b, "+ship/captives");
    assert!(captives.contains("You hold nobody aboard."), "{captives}");

    // The fee: one obligation to the Registry, settled when the account covers it. It falls due
    // at the debt floor (doc 38a review F-14: never at once), so a solvent captive pays then.
    // Its due instant is read as a ceiling (the handover was a moment earlier), and the balance is
    // read just before it, so 28 IC days of other charges do not count as the fee.
    let mut before = before;
    if paid {
        let due = tacsim::field::crew::release_fee_due(rig.tw.world());
        let _ = crate::driving_rig::to_ic(&mut rig, due - 120);
        before = balance_minor(&rig, a);
        let _ = crate::driving_rig::to_ic(&mut rig, due + 60);
    }
    let settled = seconds_until(&mut rig, 30, |rig| {
        rig.tw
            .world()
            .module_state::<mush_economy::EconomyStore>()
            .and_then(|economy| {
                economy
                    .snapshot()
                    .obligation(charge)
                    .map(|o| o.status.clone())
            })
            .is_some_and(|status| status == mush_economy::ObligationStatus::Satisfied)
    });
    let status = rig
        .tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|economy| {
            economy
                .snapshot()
                .obligation(charge)
                .map(|o| o.status.clone())
        });
    if paid {
        assert!(settled, "the fee settles: {status:?}");
        assert_eq!(before - balance_minor(&rig, a), 150_000, "1,500 cr paid");
    } else {
        assert!(!settled, "the fee stands unpaid: {status:?}");
    }

    // After the processing hour: released at the port, stamped, whatever the fee.
    let _ = crate::driving_rig::to_ic(&mut rig, release_due_ic);
    assert!(
        seconds_until(&mut rig, 30, |rig| matches!(
            crew_of(rig.tw.world(), a).map(|c| &c.location),
            Some(CrewLocation::Port { port }) if port.as_str() == "ceres"
        )),
        "A is released at Ceres: {:?}",
        crew_of(rig.tw.world(), a)
    );
    assert_eq!(
        tacsim::ic_return::last_port(rig.tw.world(), a).map(|p| p.id),
        Some("ceres"),
        "Ceres is the port A knows"
    );
    if paid {
        return;
    }
    // The unpaid fee is one of A's debts (doc 41 RS.5's `+debts`, widened for release fees),
    // and `+debts/pay` settles it once A can cover it.
    let listed = run(&mut rig.tw, a, "+debts");
    assert!(
        listed.contains(&format!("#{}", charge.0)) && listed.contains("+debts/pay"),
        "{listed}"
    );
    let from = tacsim::port::credit::own_account(rig.tw.world(), b)
        .map(|b| b.id.0)
        .expect("B's account");
    let to = tacsim::port::credit::own_account(rig.tw.world(), a)
        .map(|a| a.id.0)
        .expect("A's account");
    let _ = run(
        &mut rig.tw,
        mush_core::GOD,
        &format!("@econ/pay #{from}=#{to} 200000"),
    );
    let paid_now = run(&mut rig.tw, a, &format!("+debts/pay {}", charge.0));
    assert!(paid_now.contains("paid"), "{paid_now}");
    let status = rig
        .tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|economy| {
            economy
                .snapshot()
                .obligation(charge)
                .map(|o| o.status.clone())
        });
    assert_eq!(status, Some(mush_economy::ObligationStatus::Satisfied));
    let after = run(&mut rig.tw, a, "+debts");
    assert!(after.contains("You owe nothing."), "{after}");
}

/// **Doc 42 cut CR.3's story (e), the held case, on `at`'s ground** (Q-42-3, Q-42-8): a captor
/// who never docks. After GC.5's load, with a bound of ten IC minutes:
/// - `+sortie/extract` refuses with the time left;
/// - a restart keeps the captive's record, and the bound still runs from the capture's instant;
/// - past the bound the same verb is taken ("negotiates");
/// - after `extract_s` the captain stands at the safe port they know, `Port`, no longer in anyone's
///   custody, and the captor's `+ship/captives` is empty.
pub(crate) async fn held_past_the_bound(at: &Ground) {
    let mut rig = stocked_tuned(|cfg| {
        quick(cfg);
        cfg.field.captive_bound_s = 600;
    });
    let Loaded {
        a,
        b,
        b_carrier,
        mut rx,
        ..
    } = non_friend_loaded(&mut rig, at).await;
    let since = match crew_of(rig.tw.world(), a).map(|c| c.location.clone()) {
        Some(CrewLocation::Captured {
            carrier, since_ic, ..
        }) if carrier == b_carrier => since_ic,
        other => panic!("A is not held aboard the Red Moth: {other:?}"),
    };
    let early = run(&mut rig.tw, a, "+sortie/extract");
    assert!(
        early.contains("You are held aboard the Red Moth.") && early.contains("In about"),
        "{early}"
    );

    // A restart while captured keeps the record and the bound.
    let mut rig = crate::driving_rig::crash(rig, &mut rx, &[a, b]).await;
    let kept = crew_of(rig.tw.world(), a).map(|c| c.location.clone());
    assert!(
        matches!(kept, Some(CrewLocation::Captured { carrier, since_ic, .. }) if carrier == b_carrier && since_ic == since),
        "the restart keeps the capture: {kept:?}"
    );

    // Past the bound: the recovery service negotiates the release.
    let _ = crate::driving_rig::to_ic(&mut rig, since + 600);
    let (out, _) = command(&mut rig, a, "+sortie/extract");
    assert!(out.contains("negotiates your passage"), "{out}");
    let due = crew_of(rig.tw.world(), a)
        .and_then(|c| c.extraction.as_ref().map(|e| e.due_ic))
        .expect("the extraction is on its way");
    let _ = crate::driving_rig::to_ic(&mut rig, due);
    assert!(
        seconds_until(&mut rig, 30, |rig| matches!(
            crew_of(rig.tw.world(), a).map(|c| &c.location),
            Some(CrewLocation::Port { .. })
        )),
        "A is at a safe port: {:?}",
        crew_of(rig.tw.world(), a)
    );
    let captives = run(&mut rig.tw, b, "+ship/captives");
    assert!(captives.contains("You hold nobody aboard."), "{captives}");
}

/// **Doc 42 cut CR.4's story (d), the ejection, on `at`'s ground** (Q-42-10 to Q-42-14) with
/// `[tacsim.field].eject_capture_permille` at `permille`. The captain flies the escort from
/// inside it, its power gone (Q-42-11: no power is doomed); a stranger's mark opens a field with
/// the stranger's armed hull within reach and the stranger's carrier, the Red Moth, on the pad. In
/// the field the captain types `+sortie/eject`:
/// - the seat empties, the record is stranded with a forced extraction, and the status line names
///   the pickup;
/// - at 1,000 the stranger's party holds the cell and the draw captures: at the due instant the
///   captain is carried aboard the Red Moth, `Captured` by the stranger, who lists them; past the
///   bound (compressed to ten IC minutes) `+sortie/extract` lands them at a port;
/// - at 0 the draw never captures: the captain's second rig standing on the ground defers the
///   pickup past its due instant (Q-42-14), and the extraction lands at the port after the
///   rescue bound, the condition kept (Fit: a powerless hull hurts nobody, Q-42-13).
pub(crate) async fn eject_under_the_hold(at: &Ground, permille: u32) {
    let mut rig = stocked_tuned(|cfg| {
        quick(cfg);
        cfg.field.eject_capture_permille = permille;
        cfg.field.captive_bound_s = 600;
    });
    let (a, _, tvs) = on_pad_as_at(&mut rig, ("Pilot", 1, "Salt Moth"), at.site(), 2);
    let (escort, wing) = (tvs[0], tvs[1]);
    let ramp = ramp_cell_on(&rig, at.entry);
    launched_at(&mut rig, at.entry, a, escort);
    to_cell_on(&mut rig, at.entry, escort, (ramp.0 + 1, ramp.1));
    // The wing stands on the ground, unarmed and far off: it contests nothing, and it is the
    // rescue a pickup waits for.
    // `launched_at` launches slot 1; the wing is slot 2.
    let _ = crate::driving_rig::act_go(&mut rig, a, "+tv/2 launch go");
    let _ = crate::driving_rig::finish(&mut rig, wing, a);
    to_cell_on(&mut rig, at.entry, wing, (ramp.0 + 1, ramp.1 + 6));
    powerless(&mut rig, escort);
    seat(&mut rig, escort, a);
    crate::driving_rig::staff_hand(&mut rig, |ctx| {
        ctx.w.move_to(a, escort).expect("A climbs into the escort");
    });
    let (b, b_row, _) = on_pad_as_at(&mut rig, ("Second", 2, "Red Moth"), at.site(), 1);
    let b_carrier = ObjId(b_row.hull);
    let hull = armed_on(
        &mut rig,
        at.entry,
        "Red Hound",
        (ramp.0 - 4, ramp.1),
        90_000,
    );
    seat(&mut rig, hull, b);
    warm(&mut rig);
    let mut rx = egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();
    let (out, queued) = command(&mut rig, b, "+field/mark E=hostile");
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    crate::driving_rig::land(&mut rig, &queued[0]).expect("the promotion lands");
    assert!(
        until(&mut rig, &mut rx, &mut lines, 20, |rig, _| task(rig)
            .is_some())
        .await,
        "the field opens"
    );

    // The eject: asked, then answered at the next pulse.
    let asked = run(&mut rig.tw, a, "+sortie/eject");
    assert!(asked.contains("You reach for the release"), "{asked}");
    assert!(
        until(&mut rig, &mut rx, &mut lines, 40, |rig, _| crew_of(
            rig.tw.world(),
            a
        )
        .is_some_and(|crew| crew.awaiting_pickup()))
        .await,
        "the eject lands: {:?}",
        crew_of(rig.tw.world(), a)
    );
    let record = crew_of(rig.tw.world(), a).cloned().expect("a record");
    assert!(
        matches!(&record.location, CrewLocation::Stranded { unit, .. } if *unit == escort),
        "{record:?}"
    );
    let coming = record.extraction.clone().expect("the pickup");
    assert!(
        tacsim::field::crew::seated_in(rig.tw.world(), escort).is_none(),
        "the seat is empty"
    );
    let status = run(&mut rig.tw, a, "+sortie/condition");
    assert!(status.contains("pickup after your ejection"), "{status}");
    let again = run(&mut rig.tw, a, "+sortie/eject");
    assert!(
        again.contains("You are waiting for your pickup."),
        "{again}"
    );

    if permille == 1_000 {
        let capture = coming.capture.clone().expect("the stranger holds the cell");
        assert_eq!((capture.carrier, capture.captor), (b_carrier, b));
        let _ = crate::driving_rig::to_ic(&mut rig, coming.due_ic);
        assert!(
            seconds_until(&mut rig, 30, |rig| matches!(
                crew_of(rig.tw.world(), a).map(|c| &c.location),
                Some(CrewLocation::Captured { carrier, captor, .. })
                    if *carrier == b_carrier && *captor == b
            )),
            "A is held aboard the Red Moth: {:?}",
            crew_of(rig.tw.world(), a)
        );
        let held = run(&mut rig.tw, b, "+ship/captives");
        assert!(held.contains("held aboard the Red Moth"), "{held}");
        let since = match crew_of(rig.tw.world(), a).map(|c| c.location.clone()) {
            Some(CrewLocation::Captured { since_ic, .. }) => since_ic,
            other => panic!("{other:?}"),
        };
        let _ = crate::driving_rig::to_ic(&mut rig, since + 600);
        let (out, _) = command(&mut rig, a, "+sortie/extract");
        assert!(out.contains("negotiates your passage"), "{out}");
        let due = crew_of(rig.tw.world(), a)
            .and_then(|c| c.extraction.as_ref().map(|e| e.due_ic))
            .expect("the extraction is on its way");
        let _ = crate::driving_rig::to_ic(&mut rig, due);
    } else {
        assert!(coming.capture.is_none(), "the draw at 0 never captures");
        let _ = crate::driving_rig::to_ic(&mut rig, coming.due_ic);
        let _ = second(&mut rig);
        assert!(
            crew_of(rig.tw.world(), a).is_some_and(|crew| crew.awaiting_pickup()),
            "the wing on the ground defers the pickup: {:?}",
            crew_of(rig.tw.world(), a)
        );
        let bound = i64::from(tacsim::TacsimConfig::default().field.rescue_bound_s);
        let _ = crate::driving_rig::to_ic(&mut rig, coming.due_ic + bound);
    }
    assert!(
        seconds_until(&mut rig, 30, |rig| matches!(
            crew_of(rig.tw.world(), a).map(|c| &c.location),
            Some(CrewLocation::Port { .. })
        )),
        "A lands at a port: {:?}",
        crew_of(rig.tw.world(), a)
    );
    assert_eq!(
        crew_of(rig.tw.world(), a).map(|c| c.condition),
        Some(tacsim::field::CrewCondition::Fit),
        "an eject never hurts (Q-42-13)"
    );
}

/// **Doc 42 cut CR.5's gate, beds as a limit** (Q-42-17): three injured characters stand in
/// Ceres's arrivals hall, its medical room of two beds, their records read back at a boot. The
/// first two take the beds at the medical rate; the third is under treatment at the full time,
/// "waiting for a bed". Once the first two are fit, the third takes a freed bed, the time left
/// rescaled to the medical rate, and is fit again after it.
pub(crate) fn beds_at_ceres() {
    let rig = stocked_tuned(|cfg| {
        quick(cfg);
        cfg.field.treat_injured_s = 3_600;
        cfg.field.treat_medical_permille = 500;
    });
    let hall = crate::driving_rig::station_room(&rig.tw, "ceres_port", "room.c03_arrivals");
    let mut rig = rig;
    let who: Vec<ObjId> = ["Ash", "Birch", "Cedar"]
        .into_iter()
        .map(|name| {
            crate::driving_rig::staff_hand(&mut rig, |ctx| {
                ctx.w
                    .create(mush_core::ObjKind::Player, name, mush_core::GOD, Some(hall))
                    .expect("a character")
            })
        })
        .collect();
    let now = rig.tw.world().world_ic_now().at;
    let docs = who
        .iter()
        .map(|character| {
            let record = tacsim::field::CrewRecord {
                format: tacsim::field::crew::CREW_FORMAT,
                character: *character,
                location: CrewLocation::Port {
                    port: smol_str::SmolStr::new_static("ceres"),
                },
                condition: tacsim::field::CrewCondition::Injured,
                at_ic: now,
                organizer: None,
                extraction: None,
                treatment: None,
                resolve_permille: None,
            };
            (
                tacsim::field::crew::crew_key(*character),
                serde_json::to_value(&record).expect("encodes"),
            )
        })
        .collect();
    let mut rig = crate::driving_rig::reboot_seeding(rig, docs, &who);
    let treatment = |rig: &crate::driving_rig::Rig, character: ObjId| {
        crew_of(rig.tw.world(), character).and_then(|crew| crew.treatment.clone())
    };
    assert!(
        seconds_until(&mut rig, 10, |rig| who
            .iter()
            .all(|character| treatment(rig, *character).is_some())),
        "all three are under treatment"
    );
    let beds: Vec<bool> = who
        .iter()
        .map(|character| treatment(&rig, *character).is_some_and(|under| under.medical))
        .collect();
    assert_eq!(
        beds.iter().filter(|medical| **medical).count(),
        2,
        "two beds: {beds:?}"
    );
    let waiter = who[beds.iter().position(|medical| !medical).expect("one waits")];
    let status = run(&mut rig.tw, waiter, "+sortie/condition");
    assert!(status.contains("waiting for a bed"), "{status}");
    let full = treatment(&rig, waiter).expect("waiting").due_ic;
    let first = who
        .iter()
        .filter(|character| **character != waiter)
        .filter_map(|character| treatment(&rig, *character).map(|under| under.due_ic))
        .max()
        .expect("the beds' due instants");
    // The beds free; the waiter takes one, the time left halved.
    let _ = crate::driving_rig::to_ic(&mut rig, first);
    assert!(
        seconds_until(&mut rig, 10, |rig| treatment(rig, waiter)
            .is_some_and(|under| under.medical)),
        "the waiter takes a freed bed: {:?}",
        treatment(&rig, waiter)
    );
    let rescaled = treatment(&rig, waiter).expect("in a bed").due_ic;
    assert!(
        rescaled < full,
        "the time left is rescaled: {rescaled} < {full}"
    );
    let _ = crate::driving_rig::to_ic(&mut rig, rescaled);
    assert!(
        seconds_until(&mut rig, 10, |rig| crew_of(rig.tw.world(), waiter)
            .is_some_and(
                |crew| crew.condition == tacsim::field::CrewCondition::Fit
            )),
        "the waiter is fit again"
    );
}

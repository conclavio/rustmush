//! **Doc 36 cut GC.5 — salvage on a later visit** (Q-36-16, Q-36-19, Q-36-22, Q-36-23, Q-36-29,
//! Q-36-52's non-friend half), over the composed world (`driving`'s keyed rig on Ahuna Flank).
//!
//! - **Story (a)'s second variant**: a casualty left behind is found by a scan on a later visit
//!   and brought home by the returning party, with a crate the scan also finds picked up on the
//!   way.
//! - **The non-friend load** (rewritten by doc 42 cut CR.1, story (a′) to the load, on both
//!   grounds of `grounds_rig`): a stranger tows the casualty into their own carrier's bay; the
//!   carrier's operator has custody with a claim owed, and the pilot still inside is captured,
//!   held aboard the carrier by its operator, and stays held through the captor's next field.
//! - **The 28-day write-off**: a lease rig left out that long is Occator's alone; a stranger may
//!   still tow it, and loading it hands it back to Occator with no claim.
//!
//! **What is a fixture.** The casualty's power is destroyed by staff hand (GC.3's and GC.4's
//! precedent). In the later-visit and write-off gates staff also put it on the site's index, as
//! the field of an earlier visit left it, and drop the crate, and the crate's goods are seeded on
//! the site's spoil: nothing in play drops cargo yet (Q-36-19). Whoever returns therefore knows
//! nothing of the ground until the scan. The non-friend gate runs a real field instead, because
//! the pilot's crew record is the field's own check-in.

use mush_core::ObjId;
use tacsim::arena::ledger::LedgerEvent;
use tacsim::field::{CrewLocation, crew_of};
use tacsim::port::tv::TvLeaseStatus;
use tacsim::surface::salvage::ClaimSeam;
use tacsim::surface::site_record::{KnownItem, KnownWhat, LeftCondition};

use crate::driving_rig::{
    AHUNA, act_go, command, finish, hold_pool, launched, launched_at, on_pad, on_pad_as,
    on_pad_as_at, port_map, ramp_at, run, seed_pool, stocked_tuned, store, to_cell, to_cell_on,
    warm,
};
use crate::field_gc4::{powerless, seconds_until, towing};
use crate::field_host_rig::{
    armed_on, egress, seat, second, site_record, site_record_at, task, tune, until,
};
use crate::grounds_rig::{AHUNA_FLANK, Ground, URVARA_REACH};
use crate::site_record_rig::{staff, staff_member};

/// Short tow stages and a short extraction, on the field host's lab tempo and its default
/// disengagement (doc 43 TS.16): no body here acts inside a field, it waits for it to settle.
fn quick(cfg: &mut tacsim::TacsimConfig) {
    tune(cfg);
    cfg.field.idle_demote_s = 3_600;
    cfg.field.extract_s = 600;
    cfg.recovery.inspect_s = 30;
    cfg.recovery.attach_s = 30;
    cfg.recovery.load_s = 60;
}

/// **Move the world on to IC `target` without firing a tick** (`vehicles.rs`'s `skip_days`): the
/// next fire is the first the jump sees.
pub(crate) fn skip_to_ic(rig: &mut crate::driving_rig::Rig, target: i64) {
    let w = rig.tw.world();
    let rate = i64::from(w.world_timeline().current().rate.max(1));
    let ahead = target.saturating_sub(w.world_ic_now().at).max(0);
    let t = w.now().saturating_add((ahead + rate - 1) / rate);
    rig.tw.set_now(t);
}

/// The ramp's site cell.
fn ramp_cell(rig: &crate::driving_rig::Rig) -> (i64, i64) {
    let (rx, ry, _) = ramp_at(&rig.tw, AHUNA);
    (rx.div_euclid(1_000), ry.div_euclid(1_000))
}

/// The tow attached: `tug`'s `+tv/<slot> tow go` filed and landed, then on to the attachment.
fn attach(rig: &mut crate::driving_rig::Rig, who: ObjId, slot: u8, tug: ObjId, casualty: ObjId) {
    let (out, queued) = command(rig, who, &format!("+tv/{slot} tow go"));
    assert_eq!(queued.len(), 1, "one tow start: {out}");
    crate::driving_rig::land(rig, &queued[0]).expect("the tow start lands");
    assert!(
        seconds_until(rig, 120, |rig| towing(rig, tug) == Some(casualty)),
        "the tow attaches"
    );
}

/// The load filed and landed: on to the casualty's leaving the site's index.
fn load(rig: &mut crate::driving_rig::Rig, who: ObjId, slot: u8, tug: ObjId, casualty: ObjId) {
    let (out, queued) = command(rig, who, &format!("+tv/{slot} load go"));
    assert_eq!(queued.len(), 1, "one load start: {out}");
    crate::driving_rig::land(rig, &queued[0]).expect("the load start lands");
    assert!(
        seconds_until(rig, 120, |rig| site_record(rig).is_some_and(|record| {
            record.tows.act_of(tug).is_none() && record.units.iter().all(|l| l.unit != casualty)
        })),
        "the load lands"
    );
}

/// **GC.5's gate: story (a)'s second variant** (Q-36-19, Q-36-23). A casualty and a crate lie on
/// Ahuna Flank, left by an earlier visit, and the captain knows of neither. The wingman's scan from
/// the ramp marks both into the captain's own `known` row and says so; it picks the crate up into
/// its hopper (the crate's goods move from the site's spoil, nothing is made); it tows the
/// casualty, drives the tow to the ramp, and loads it into the captain's own bay. The casualty is
/// home: in the bay, off the index, its epoch closed, in nobody's custody.
#[test]
#[allow(clippy::too_many_lines)]
fn gc5_a_casualty_left_behind_is_found_by_a_scan_and_brought_home() {
    let mut rig = stocked_tuned(quick);
    let (a, row, tvs) = on_pad(&mut rig, 2);
    let carrier = ObjId(row.hull);
    let (escort, wingman) = (tvs[0], tvs[1]);
    let ramp = ramp_cell(&rig);
    launched(&mut rig, a, escort);
    let lying = (ramp.0 + 4, ramp.1);
    to_cell(&mut rig, escort, lying);
    powerless(&mut rig, escort);
    // The earlier visit's leavings: the casualty on the index, a crate beside it, its goods on
    // the site's spoil.
    let s = staff_member(&mut rig);
    staff(
        &mut rig,
        s,
        &format!(
            "@tacsim/site {AHUNA} leave #{} casualty {},{}",
            escort.0, lying.0, lying.1
        ),
    );
    let crate_at = (ramp.0 + 2, ramp.1);
    staff(
        &mut rig,
        s,
        &format!(
            "@tacsim/site {AHUNA} drop crate 2 metal_ore {},{}",
            crate_at.0, crate_at.1
        ),
    );
    let spoil = mush_economy::VaultId(
        port_map(&rig.tw, AHUNA)
            .site
            .as_ref()
            .and_then(|site| site.spoil_vault)
            .expect("Ahuna's spoil vault"),
    );
    seed_pool(
        &mut rig,
        spoil,
        "metal_ore",
        2,
        mush_inventory::LotId(9_501),
    );
    let record = site_record(&mut rig).expect("the site record");
    assert!(
        record.known_by(a).is_empty(),
        "the captain knows nothing yet"
    );

    // The scan (Q-36-23): from the ramp, it marks both into the captain's own row, and says so.
    let _ = act_go(&mut rig, a, "+tv/2 scan go");
    let heard = finish(&mut rig, wingman, a);
    assert!(
        heard.contains("also marks what lies on the ground in reach: a casualty, a crate"),
        "{heard}"
    );
    let record = site_record(&mut rig).expect("the site record");
    let known = record.known_by(a);
    assert!(
        known.contains(&KnownItem(KnownWhat::Casualty, lying.0, lying.1))
            && known.contains(&KnownItem(KnownWhat::Crate, crate_at.0, crate_at.1)),
        "the scan marked both: {known:?}"
    );

    // The pickup (Q-36-19): the crate's goods go from the spoil into the hopper.
    to_cell(&mut rig, wingman, crate_at);
    let preview = run(&mut rig.tw, a, "+tv/2 pickup");
    assert!(
        preview.contains("can take a crate of 2 metal_ore aboard"),
        "{preview}"
    );
    let (out, queued) = command(&mut rig, a, "+tv/2 pickup go");
    assert_eq!(queued.len(), 1, "one pickup: {out}");
    assert_eq!(
        queued[0].action.key().id.as_str(),
        tacsim::surface::tow::TOW_ACTION
    );
    crate::driving_rig::land(&mut rig, &queued[0]).expect("the pickup lands");
    let hopper =
        tacsim::port::vehicle::hopper_of(rig.tw.world(), wingman).expect("the wingman's hopper");
    assert_eq!(
        hold_pool(&mut rig.tw, hopper, "metal_ore"),
        2,
        "in the hopper"
    );
    assert_eq!(
        hold_pool(&mut rig.tw, spoil, "metal_ore"),
        0,
        "off the spoil"
    );
    assert!(
        site_record(&mut rig).is_some_and(|record| record.objects.is_empty()),
        "the crate is off the ground"
    );
    let again = run(&mut rig.tw, a, "+tv/2 pickup");
    assert!(again.contains("No crate lies within a cell"), "{again}");

    // The tow, home: attach, drive the tow to the ramp, load into the captain's own bay.
    to_cell(&mut rig, wingman, (lying.0 + 1, lying.1));
    attach(&mut rig, a, 2, wingman, escort);
    let _ = act_go(
        &mut rig,
        a,
        &format!("+tv/2 drive {},{} go", ramp.0 + 1, ramp.1),
    );
    let _ = finish(&mut rig, wingman, a);
    assert_eq!(
        crate::field_gc4::cell_of(&rig, escort),
        crate::field_gc4::cell_of(&rig, wingman),
        "the casualty follows its tug"
    );
    let preview = run(&mut rig.tw, a, "+tv/2 load");
    assert!(
        preview.contains("into the Salt Moth's bay") && !preview.contains("custody"),
        "{preview}"
    );
    load(&mut rig, a, 2, wingman, escort);
    assert!(
        tacsim::surface::tow::in_bay_of(rig.tw.world(), escort, carrier),
        "the casualty is home"
    );
    let record = site_record(&mut rig).expect("the site record");
    assert!(
        record.custody.is_empty(),
        "a friendly load is nobody's custody"
    );
    let escort_record = crate::driving_rig::record_of(&mut rig.tw, escort).expect("its rig record");
    assert!(escort_record.sortie.is_none(), "its epoch is closed");
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
async fn non_friend_load_captures_the_crew(at: &Ground) {
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

/// **GC.5's gate at 256** on Ahuna Flank ([`non_friend_load_captures_the_crew`]).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gc5_a_non_friend_load_takes_custody_and_captures_the_crew() {
    non_friend_load_captures_the_crew(&AHUNA_FLANK).await;
}

/// **GC.5's gate at 512** on Urvara Reach ([`non_friend_load_captures_the_crew`]).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gc5_a_non_friend_load_takes_custody_and_captures_the_crew_at_512() {
    non_friend_load_captures_the_crew(&URVARA_REACH).await;
}

/// **GC.5's gate: the 28-day write-off and Occator's hand-back** (Q-36-22, Q-36-29). The
/// captain's leased rig lies disabled on the index. Short of 28 IC days nothing happens; past
/// them the surface tick writes it off: the lease ended, the deposit Occator's, the record
/// written off. A stranger may still tow it, and the load's preview says it goes back to Occator;
/// the load puts it in Occator's shed, in nobody's custody, off the index and its epoch closed.
#[test]
#[allow(clippy::too_many_lines)]
fn gc5_a_lease_rig_out_28_days_is_written_off_and_handed_back_when_loaded() {
    let mut rig = stocked_tuned(quick);
    let (a, _, tvs) = on_pad(&mut rig, 1);
    let escort = tvs[0];
    let ramp = ramp_cell(&rig);
    launched(&mut rig, a, escort);
    to_cell(&mut rig, escort, (ramp.0 + 1, ramp.1));
    powerless(&mut rig, escort);
    let s = staff_member(&mut rig);
    staff(
        &mut rig,
        s,
        &format!(
            "@tacsim/site {AHUNA} leave #{} casualty {},{}",
            escort.0,
            ramp.0 + 1,
            ramp.1
        ),
    );
    let lease = store(&rig.tw)
        .tv_lease_on(escort)
        .cloned()
        .expect("the escort is leased");
    let left = site_record(&mut rig)
        .and_then(|record| record.units.first().map(|row| row.left_ic))
        .expect("indexed");
    let day = 86_400;
    let bar = tacsim::port::tv::WRITE_OFF_LEFT_IC_DAYS * day;

    // A day short: still the lessee's.
    skip_to_ic(&mut rig, left + bar - day);
    let _ = second(&mut rig);
    assert!(
        !tacsim::port::tv::written_off(rig.tw.world(), escort),
        "not yet"
    );
    // Past the 28 days: written off on the tick.
    skip_to_ic(&mut rig, left + bar + 60);
    let done = seconds_until(&mut rig, 120, |rig| {
        tacsim::port::tv::written_off(rig.tw.world(), escort)
    });
    let units = site_record(&mut rig).map(|record| record.units);
    assert!(
        done,
        "written off: IC now {} (left {left}, bar {bar}), leases {:?}, units {:?}",
        rig.tw.world().world_ic_now().at,
        store(&rig.tw)
            .tv_leases_on(escort)
            .cloned()
            .collect::<Vec<_>>(),
        units
    );
    let ended = store(&rig.tw)
        .tv_leases_on(escort)
        .find(|record| record.lease == lease.lease)
        .cloned()
        .expect("the record");
    assert_eq!(ended.status, TvLeaseStatus::Returned);
    assert!(ended.written_off);
    let hold = rig
        .tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|economy| {
            economy
                .snapshot()
                .hold(mush_economy::HoldId(ended.hold))
                .map(|hold| hold.status.clone())
        })
        .expect("the deposit hold");
    assert!(
        matches!(hold, mush_economy::HoldStatus::Consumed { .. }),
        "the deposit is Occator's: {hold:?}"
    );

    // A stranger may tow it; loading it hands it back to Occator, with no claim.
    let (b, b_row, b_tvs) = on_pad_as(&mut rig, "Second", 2, "Red Moth", 1);
    let b_carrier = ObjId(b_row.hull);
    let tug = b_tvs[0];
    launched(&mut rig, b, tug);
    to_cell(&mut rig, tug, (ramp.0 + 1, ramp.1));
    attach(&mut rig, b, 1, tug, escort);
    let preview = run(&mut rig.tw, b, "+tv/1 load");
    assert!(
        preview.contains("Occator write-off: loaded, it goes back to Occator, with no claim"),
        "{preview}"
    );
    load(&mut rig, b, 1, tug, escort);
    assert!(
        tacsim::port::tv::in_shed(rig.tw.world(), escort),
        "back in Occator's shed"
    );
    assert!(
        !tacsim::surface::tow::in_bay_of(rig.tw.world(), escort, b_carrier),
        "not in the stranger's bay"
    );
    let record = site_record(&mut rig).expect("the site record");
    assert!(record.custody.is_empty(), "no custody, so no claim");
    let escort_record = crate::driving_rig::record_of(&mut rig.tw, escort).expect("its rig record");
    assert!(escort_record.sortie.is_none(), "its epoch is closed");
    assert!(escort_record.excursion.is_none(), "its excursion is closed");
}

/// **Doc 36a §11 row 4, rewritten by doc 42 cut CR.1: the captor's field close keeps the captive**
/// (Q-42-3) on `at`'s ground. After the non-friend load A is held in the Red Moth's bay room,
/// physically aboard the Red Moth and so among the souls aboard it. The stranger's mark at a
/// bystander's hull opens a second field, and the Red Moth is admitted with A among the souls
/// aboard its row. Its close checks the Red Moth's souls in, and once checked A in as the
/// captor's crew; a record in custody is left alone now: A is still held by the stranger from
/// the same instant, never re-crewed, and the hull stays with the carrier.
async fn captors_field_close_keeps_the_captive(at: &Ground) {
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

/// **The twin at 256** on Ahuna Flank ([`captors_field_close_keeps_the_captive`]).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gc5_review_the_captors_field_close_keeps_the_captive() {
    captors_field_close_keeps_the_captive(&AHUNA_FLANK).await;
}

/// **The twin at 512** on Urvara Reach ([`captors_field_close_keeps_the_captive`]).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gc5_review_the_captors_field_close_keeps_the_captive_at_512() {
    captors_field_close_keeps_the_captive(&URVARA_REACH).await;
}

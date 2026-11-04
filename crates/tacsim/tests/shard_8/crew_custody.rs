//! **Doc 42 — crew custody and capture**, the stories grown cut by cut (§7a; Q-42-25), over the
//! composed world (`driving`'s keyed rig) on both grounds of `grounds_rig`, in shard_8 for its
//! room (doc 43 rule 2.3 #3, #14). Story (a′)'s load itself is GC.5's rewritten gate, hoisted to
//! `custody_rig`: `field_gc5` runs it at 256 beside GC.5, and this file at 512.
//!
//! - **Cut CR.1, story (a′) to the load at 512** (Q-42-4, Q-42-16; Q-42-3 for the twin): GC.5's
//!   gate and its review twin on Urvara Reach (`custody_rig::non_friend_load_captures_the_crew`,
//!   `custody_rig::captors_field_close_keeps_the_captive`).
//! - **Cut CR.1, the hired NPC's exit** (Q-42-5, Q-42-26): a captain hires Ilya Mendes at
//!   Ceres's Slack Line through `+sortie/crew` and `+sortie/hire`; the NPC flies the escort, which
//!   a stranger's rig loads into the stranger's carrier. At the load the NPC's seat is vacated, the
//!   NPC stands in the Slack Line again and the captain is told; the contract still stands, so the
//!   NPC is not on offer. The tow tick submits `tacsim_crew_captured` as the captain until it
//!   lands: the contract is gone, the owed exit forgotten, and the NPC is on offer again.
//! - **Cut CR.1, the captor's release** (Q-42-9): after GC.5's load the stranger's
//!   `+ship/captives` names the captain and `+ship/release` lets them go on the site, a passenger
//!   aboard the stranger's carrier; the list is empty after.
//!
//! **What is a fixture.** The NPC is seated by a staff hand (`field_raiders_rig::seat_npc`, the
//! billet a DT flight's activation writes), and the escort, powerless by staff hand, is put on the
//! site's index as an earlier field would have left it (GC.5's later-visit precedent): no field is
//! needed for a load. The hire, the tow, the load and the release are the production command
//! paths.

use mush_core::ObjId;
use tacsim::field::{CrewLocation, crew_of, npc_exit_of};

use crate::custody_rig::{Loaded, attach_on, non_friend_loaded, powerless, quick, ramp_cell_on};
use crate::driving_rig::{
    Rig, command, launched_at, on_pad_as_at, on_pad_with_at, run, said, staff_hand, station_room,
    stocked_tuned, teleport, to_cell_on,
};
use crate::field_host_rig::{second, site_record_at};
use crate::grounds_rig::{AHUNA_FLANK, Ground, URVARA_REACH};
use crate::site_record_rig::{staff, staff_member};

/// The content's mining NPC at Ceres (`content/crew/ceres.json`).
const MENDES: &str = "Ilya Mendes";

/// The NPC named `name`.
fn npc_named(rig: &Rig, name: &str) -> ObjId {
    rig.tw
        .world()
        .iter()
        .find(|object| object.kind == mush_core::ObjKind::Npc && object.name.as_str() == name)
        .map(|object| object.id)
        .unwrap_or_else(|| panic!("{name} stands in the world"))
}

/// **Hire `npc` as `who`** at the room they share, through the offer `+sortie/crew` prints.
fn hire(rig: &mut Rig, who: ObjId, npc: ObjId) {
    let offers = run(&mut rig.tw, who, "+sortie/crew");
    let accept = offers
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix("Accept: ")
                .filter(|line| line.contains(&format!("#{}=", npc.0)))
        })
        .unwrap_or_else(|| panic!("{npc:?} is on offer: {offers}"))
        .to_owned();
    let (out, queued) = command(rig, who, &accept);
    assert_eq!(queued.len(), 1, "one hire: {out}");
    crate::driving_rig::land_composed(rig, &queued[0]).expect("the hire lands");
}

/// **Seconds until `done` holds, at most `limit`**, every tick's submissions landed: what `who`
/// heard from the ticks meanwhile, and whether it held.
fn hearing_until(
    rig: &mut Rig,
    who: ObjId,
    limit: u32,
    mut done: impl FnMut(&mut Rig) -> bool,
) -> (bool, String) {
    let mut heard = String::new();
    for _ in 0..limit {
        if done(rig) {
            return (true, heard);
        }
        let t = rig.tw.world().now().saturating_add(1);
        rig.tw.set_now(t);
        let events = rig.tw.fire_due(t.saturating_mul(1_000));
        heard.push_str(&said(&rig.tw, &events, who));
        heard.push('\n');
        for submit in crate::driving_rig::drain(&mut rig.tw) {
            if let Err(why) = crate::driving_rig::land_composed(rig, &submit) {
                heard.push_str(&format!("refused {}: {why}\n", submit.action.key().id));
            }
        }
    }
    (done(rig), heard)
}

/// Whether `npc` is on offer to `who` where `who` stands.
fn on_offer(rig: &mut Rig, who: ObjId, npc: ObjId) -> bool {
    run(&mut rig.tw, who, "+sortie/crew").contains(&format!("+sortie/hire #{}=", npc.0))
}

/// **CR.1's NPC exit on `at`'s ground** (this module's head).
#[allow(clippy::too_many_lines)] // one story, in order
fn npc_exit_on(at: &Ground) {
    let mut rig = stocked_tuned(quick);
    let mendes = npc_named(&rig, MENDES);
    let pool = rig
        .tw
        .world()
        .get(mendes)
        .and_then(|object| object.location)
        .expect("Mendes stands in the Slack Line");
    assert_eq!(
        pool,
        station_room(&rig.tw, "ceres_port", "room.c10_slack_line")
    );
    // The captain hires Mendes in the Slack Line before sailing; the contract is K3 state.
    let (a, _, tvs) = on_pad_with_at(&mut rig, ("Pilot", 1, "Salt Moth"), at.site(), |rig, a| {
        teleport(&mut rig.tw, a, pool);
        hire(rig, a, mendes);
        assert!(!on_offer(rig, a, mendes), "a hired NPC is not on offer");
        crate::driving_rig::at_counter(rig, a);
        let leased = run(&mut rig.tw, a, "+tv/lease go");
        assert!(leased.contains("leases you"), "{leased}");
    });
    let escort = tvs[0];
    let contract = run(&mut rig.tw, a, &format!("+sortie/contract #{}", mendes.0));
    assert!(contract.contains("Crew contract"), "{contract}");

    // Mendes flies the escort, which lies powerless on the index beside the ramp.
    let ramp = ramp_cell_on(&rig, at.entry);
    launched_at(&mut rig, at.entry, a, escort);
    let lying = (ramp.0 + 1, ramp.1);
    to_cell_on(&mut rig, at.entry, escort, lying);
    powerless(&mut rig, escort);
    staff_hand(&mut rig, |ctx| {
        ctx.w.move_to(mendes, escort).expect("Mendes climbs in");
    });
    crate::field_raiders_rig::seat_npc(&mut rig, escort, mendes);
    let s = staff_member(&mut rig);
    staff(
        &mut rig,
        s,
        &format!(
            "@tacsim/site {} leave #{} casualty {},{}",
            at.entry, escort.0, lying.0, lying.1
        ),
    );

    // A stranger's rig tows the escort into the stranger's carrier.
    let (b, b_row, b_tvs) = on_pad_as_at(&mut rig, ("Second", 2, "Red Moth"), at.site(), 1);
    let b_carrier = ObjId(b_row.hull);
    let tug = b_tvs[0];
    launched_at(&mut rig, at.entry, b, tug);
    to_cell_on(&mut rig, at.entry, tug, (ramp.0 + 1, ramp.1 + 1));
    let _ = second(&mut rig);
    attach_on(&mut rig, at.entry, (b, 1), tug, escort);
    let (out, queued) = command(&mut rig, b, "+tv/1 load go");
    assert_eq!(queued.len(), 1, "one load start: {out}");
    crate::driving_rig::land(&mut rig, &queued[0]).expect("the load start lands");
    let (loaded, heard) = hearing_until(&mut rig, a, 120, |rig| {
        site_record_at(rig, at.entry).is_some_and(|record| {
            record.tows.act_of(tug).is_none() && record.units.iter().all(|l| l.unit != escort)
        })
    });
    assert!(loaded, "the load lands: {heard}");
    assert!(tacsim::surface::tow::in_bay_of(
        rig.tw.world(),
        escort,
        b_carrier
    ));

    // The world half at once (Q-42-5): out of the seat, back in the pool room, the captain told.
    assert_eq!(
        rig.tw
            .world()
            .get(mendes)
            .and_then(|object| object.location),
        Some(pool),
        "Mendes is back in the Slack Line"
    );
    assert!(
        rig.tw
            .world()
            .module_state::<mush_sim::SpatialState>()
            .and_then(|state| state.assignments.get(&escort))
            .is_none_or(|host| host.seats.values().all(|seat| seat.actor != mendes)),
        "the seat is vacated"
    );
    assert!(
        crew_of(rig.tw.world(), mendes).is_none(),
        "an NPC keeps no crew record"
    );
    assert!(
        heard.contains("their contract with you ends") && heard.contains("Slack Line"),
        "{heard}"
    );
    // The roster's half (Q-42-26): the tick submits as the captain until the ledger shows it (the
    // load's own pass may already have landed it).
    let (landed, heard) = hearing_until(&mut rig, a, 30, |rig| {
        npc_exit_of(rig.tw.world(), mendes).is_none()
    });
    assert!(landed, "the owed exit lands: {heard}");
    let ended = run(&mut rig.tw, a, &format!("+sortie/contract #{}", mendes.0));
    assert!(ended.contains("Crew or contract unavailable"), "{ended}");
    let captain = rig.tw.spawn_player("Third");
    teleport(&mut rig.tw, captain, pool);
    assert!(
        on_offer(&mut rig, captain, mendes),
        "Mendes is on offer again"
    );
    let record = site_record_at(&mut rig, at.entry).expect("the site record");
    assert!(
        record
            .unlawful
            .iter()
            .all(|act| act.kind != tacsim::surface::claim::UnlawfulKind::Capture),
        "an NPC is never a captive, and unowned ground writes no evidence: {:?}",
        record.unlawful
    );
}

/// **CR.1's NPC exit at 256** on Ahuna Flank ([`npc_exit_on`]).
#[test]
fn cr1_a_captured_hulls_hired_npc_goes_back_to_the_pool_and_off_the_roster() {
    npc_exit_on(&AHUNA_FLANK);
}

/// **CR.1's NPC exit at 512** on Urvara Reach ([`npc_exit_on`]).
#[test]
fn cr1_a_captured_hulls_hired_npc_goes_back_to_the_pool_and_off_the_roster_at_512() {
    npc_exit_on(&URVARA_REACH);
}

/// **CR.1's release on the site** (Q-42-9), after GC.5's load on Ahuna Flank: the stranger's
/// `+ship/captives` names the captain; a release naming nobody held refuses; `+ship/release`
/// sets the captain free aboard the Red Moth, a passenger (`Carrier`), told so; the list is empty.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cr1_the_captor_releases_a_captive_on_the_site() {
    let mut rig = stocked_tuned(quick);
    let Loaded {
        a, b, b_carrier, ..
    } = non_friend_loaded(&mut rig, &AHUNA_FLANK).await;
    let listed = run(&mut rig.tw, b, "+ship/captives");
    assert!(listed.contains(&format!("(#{})", a.0)), "{listed}");
    let nobody = run(&mut rig.tw, b, "+ship/release #1");
    assert!(
        nobody.contains("You hold nobody aboard by that name."),
        "{nobody}"
    );
    let stranger = run(&mut rig.tw, a, "+ship/captives");
    assert!(
        stranger.contains("You are held aboard"),
        "a captive runs no +ship: {stranger}"
    );
    let events = rig.tw.run(b, &format!("+ship/release #{}", a.0));
    let released = said(&rig.tw, &events, b);
    assert!(released.contains("You release"), "{released}");
    let heard = said(&rig.tw, &events, a);
    assert!(
        heard.contains("releases you aboard the Red Moth"),
        "{heard}"
    );
    let crew = crew_of(rig.tw.world(), a).cloned().expect("A's record");
    assert_eq!(crew.location, CrewLocation::Carrier { hull: b_carrier });
    let empty = run(&mut rig.tw, b, "+ship/captives");
    assert!(empty.contains("You hold nobody aboard."), "{empty}");
    let condition = run(&mut rig.tw, a, "+sortie/condition");
    assert!(!condition.contains("held aboard"), "{condition}");
}

/// **GC.5's gate at 512** on Urvara Reach (`custody_rig::non_friend_load_captures_the_crew`;
/// `field_gc5` runs it at 256).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cr1_a_non_friend_load_takes_custody_and_captures_the_crew_at_512() {
    crate::custody_rig::non_friend_load_captures_the_crew(&URVARA_REACH).await;
}

/// **GC.5's review twin at 512** on Urvara Reach (`custody_rig::captors_field_close_keeps_the_captive`;
/// `field_gc5` runs it at 256).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cr1_the_captors_field_close_keeps_the_captive_at_512() {
    crate::custody_rig::captors_field_close_keeps_the_captive(&URVARA_REACH).await;
}

/// **CR.2's handover at 256, the fee paid** (`custody_rig::handover_at_ceres`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cr2_the_captor_docks_at_ceres_and_the_captive_is_handed_over_and_released() {
    crate::custody_rig::handover_at_ceres(&AHUNA_FLANK, true).await;
}

/// **CR.2's handover at 512, the fee unpaid** (`custody_rig::handover_at_ceres`): the release
/// never waits on it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cr2_an_unpaid_release_fee_stands_and_the_captive_is_released_anyway_at_512() {
    crate::custody_rig::handover_at_ceres(&URVARA_REACH, false).await;
}

/// **CR.3's held case at 256** (`custody_rig::held_past_the_bound`): a captor who never docks.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cr3_a_captive_extracts_after_the_bound_and_a_restart_keeps_it() {
    crate::custody_rig::held_past_the_bound(&AHUNA_FLANK).await;
}

/// **CR.3's held case at 512** (`custody_rig::held_past_the_bound`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cr3_a_captive_extracts_after_the_bound_and_a_restart_keeps_it_at_512() {
    crate::custody_rig::held_past_the_bound(&URVARA_REACH).await;
}

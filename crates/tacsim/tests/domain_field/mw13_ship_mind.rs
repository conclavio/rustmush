//! **Doc 44 unit MW.13: a player's own mind sealed on a ship** (Q-44-26; review findings 4, 17,
//! 18 and 25).
//!
//! The filed-mind stories run on the combined M8 world (the production `LaunchComposition`,
//! `m8_world`), because a sealed mind is the Roll's claim and only an Active launch composes
//! the claim path. Each takes one genuine Registry rating, so the CI dispatch runs them. The
//! stories that need no claim run on the fitted excursions rig in the default sweep.
//!
//! **What stands in.** No ship ships with a core and nothing in play fits one to a ship (the
//! chartered shop fits a hull that stands in it). Staff bind the pilot's chartered Dray to
//! the Dray's governed Class II root (`@unit/remove`, `@unit/create`), as doc 17h's stories
//! bound a rig's before the fitting shop was on a launch. Staff also top the pilot's wallet up
//! for the filing fee. Everything else is the pilot's own typing and the world's own ticks.

use mush_core::ObjId;
use tacsim::definitions::core_replacement::{SHIP_II, SHIP_II_UNGOVERNED};
use tacsim::port::epoch::{EpochKey, ShipEpochRecord};
use tacsim::port::ship_mind::{self, Aboard};
use tacsim::space::{BackupAct, PicketInspectionStatus, SpaceBubbleBook};
use tacsim::surface::excursion::ClaimCursor;

use crate::excursions::{Rig, command, docked_dray, run};
use crate::m8_world::{M8World, fires, m8_world_manned};

/// **The pilot's own script.** In a bubble, told to heave to, it locks the ship that told it
/// and burns to end its drift with everything its drive gives; hove to or free, it cuts its
/// drive. Outside a bubble it makes for port. Its trace words are its own. A cold hull (the
/// Registry's trials begin cold) it leaves cold, so in a trial it asks nothing.
fn keeper() -> String {
    "fn think(v, c) { \
       if v.power != \"online\" { return; } \
       let s = space(v); \
       if s == () { speed(1000); trace(\"KEEPER_MAKE_FOR_PORT\"); return; } \
       if s.stop == \"hailed\" && s.stop_by != () { \
         lock(s.stop_by); \
         let by = space_contact(v, s.stop_by); \
         if by != () { \
           let dx = by.vx_cms - s.vx_cms; let dy = by.vy_cms - s.vy_cms; let dz = by.vz_cms - s.vz_cms; \
           if isqrt(dx * dx + dy * dy + dz * dz) > 0 { \
             helm(course_of(dx, dy, dz)); speed(max(s.accel_cms2, 1)); \
             trace(\"KEEPER_HEAVE_TO\"); return; \
           } \
         } \
       } \
       hold(); \
       if v.held { trace(\"KEEPER_STAND_FOR_CUSTOMS\"); } else { trace(\"KEEPER_STAND\"); } \
     }"
    .to_owned()
}

/// The hull's epoch record, with its claim cursor.
fn record(rig: &mut Rig, hull: ObjId) -> Option<ShipEpochRecord> {
    let handles = crate::excursions::handles(&rig.tw);
    crate::excursions::typed(&mut rig.tw, &handles.families.epochs, &EpochKey(hull))
}

/// The hull's claim cursor.
fn cursor(rig: &mut Rig, hull: ObjId) -> Option<ClaimCursor> {
    record(rig, hull).and_then(|record| record.claim.map(|claim| *claim))
}

/// Drop every connection `who` holds, uncleanly, so the presence hook reads a lost link.
fn drop_link(rig: &mut Rig, who: ObjId) {
    let conns: Vec<u64> = (1..=64)
        .filter(|conn| rig.tw.engine().actor_of(*conn) == Some(who))
        .collect();
    assert!(!conns.is_empty(), "{who} holds a connection");
    for conn in conns {
        rig.tw.engine_mut().step(
            mush_core::Inbound::ConnDown {
                conn,
                actor: who,
                reason: mush_core::DisconnectKind::Lost,
            },
            &mut mush_core::Outbox::new(),
            &mut mush_core::Effects::new(),
        );
    }
}

/// One wall second on, everything due fired and what it queued landed.
fn one_second(rig: &mut Rig) {
    let t = rig.tw.world().now().saturating_add(1);
    rig.tw.set_now(t);
    let _ = rig.tw.fire_due(t.saturating_mul(1_000));
    let queued = crate::excursions::drain(&mut rig.tw);
    crate::excursions::land_quietly(rig, &queued);
}

/// Wait the link-loss grace out at the play rate, and one second more for the mind's first
/// think to land (doc 44 unit MW.11), then go back to the stories' rate.
fn wait_out_the_grace(rig: &mut Rig) {
    crate::excursions::set_rate(&mut rig.tw, 24);
    let past = rig
        .tw
        .world()
        .now()
        .saturating_add(tacsim::space::SPACE_LINKDEAD_GRACE_WALL_S + 10);
    rig.tw.set_now(past);
    let _ = rig.tw.fire_due(past.saturating_mul(1_000));
    let queued = crate::excursions::drain(&mut rig.tw);
    crate::excursions::land_quietly(rig, &queued);
    one_second(rig);
    crate::excursions::set_rate(&mut rig.tw, 3_600);
}

/// What a story reads of a hull at the Kirkwood picket.
#[derive(Clone, Debug, PartialEq, Eq)]
struct AtThePicket {
    /// Whether her lock is the customs corvette.
    locks_the_corvette: bool,
    /// Her lock, whatever it is.
    lock: Option<String>,
    /// Her commanded thrust, mm/s².
    thrust: [i64; 3],
    /// Her speed against the corvette, mm/s.
    drift: i64,
    /// What customs have made of her.
    status: PicketInspectionStatus,
    /// The last word her mind traced.
    trace: Option<String>,
    /// What the seat last did for her.
    act: Option<BackupAct>,
}

fn at_the_picket(rig: &Rig, hull: ObjId) -> Option<AtThePicket> {
    let book = rig
        .tw
        .world()
        .module_state::<SpaceBubbleBook>()
        .expect("the space book");
    let active = book.active_for_hull(hull)?;
    let picket = active.picket.as_ref()?;
    let ship = active.bubble.ships.get(active.carrier_ship_id_for(hull))?;
    let corvette = active.bubble.ships.get(&picket.corvette_ship_id)?;
    let linkdead = book.linkdead_state(hull);
    Some(AtThePicket {
        locks_the_corvette: ship.locked_target_id.as_deref()
            == Some(picket.corvette_ship_id.as_str()),
        lock: ship.locked_target_id.clone(),
        thrust: ship.thrust_mmps2,
        drift: tacsim::space::relative_speed_3d_mmps(ship.vel_mmps, corvette.vel_mmps),
        status: picket.inspection_status_for(hull),
        trace: linkdead.and_then(|state| state.mind_last_trace.clone()),
        act: linkdead.map(|state| state.backup_act),
    })
}

/// **File for Stickney and lose the link on the corridor**: past the grace a mind has her
/// helm. Answers the IC instant her first leg ends, which is where the Kirkwood picket's
/// bubble opens on her.
fn sail_and_lose_the_link(rig: &mut Rig, pilot: ObjId, hull: ObjId) -> i64 {
    crate::excursions::set_rate(&mut rig.tw, 3_600);
    let ticket = crate::excursions::file_and_launch(rig, pilot, hull, "+ship/undock stickney");
    let boundary = ticket.plan.depart_ic + i64::from(ticket.plan.legs[0].secs);
    drop_link(rig, pilot);
    wait_out_the_grace(rig);
    let held = rig
        .tw
        .world()
        .module_state::<SpaceBubbleBook>()
        .and_then(|book| book.linkdead_state(hull))
        .cloned()
        .expect("her link-loss record");
    assert!(held.backup_has_helm(), "past the grace a mind has her helm");
    boundary
}

/// Come to the end of her first leg, where the picket's bubble opens on her.
fn arrive_at_the_picket(rig: &mut Rig, hull: ObjId, boundary: i64) {
    let _ = crate::excursions::to_ic(rig, boundary);
    assert!(
        at_the_picket(rig, hull).is_some(),
        "the Kirkwood picket's bubble opens on her"
    );
}

/// [`sail_and_lose_the_link`], then [`arrive_at_the_picket`].
fn to_the_picket_with_no_link(rig: &mut Rig, pilot: ObjId, hull: ObjId) {
    let boundary = sail_and_lose_the_link(rig, pilot, hull);
    arrive_at_the_picket(rig, hull, boundary);
}

/// Step a second at a time until `done` reads true of her at the picket, or she has left it.
fn until_at_the_picket(
    rig: &mut Rig,
    hull: ObjId,
    seconds: u32,
    mut done: impl FnMut(&AtThePicket) -> bool,
) -> Option<AtThePicket> {
    for _ in 0..seconds {
        one_second(rig);
        let now = at_the_picket(rig, hull)?;
        if done(&now) {
            return Some(now);
        }
    }
    at_the_picket(rig, hull)
}

/// **Doc 44 unit MW.13: what the verb refuses with no claim in the world, and what the house
/// default does at the picket** (the fitted excursions rig; default sweep).
///
/// 1. A Dray as she ships carries no core: `+ship/mind` refuses her by that.
/// 2. Bound to the Dray's governed Class II root she is still a Dray to every port verb (her
///    status, her plan, her passage), and reads a core.
/// 3. On that governed core an unfiled draft is refused, by the sentence a rig's launch says
///    (finding 4); on the ungoverned twin a draft is refused as not built for a ship.
/// 4. A filed word is refused where no claim path is composed: nothing would carry the seal.
/// 5. With nothing sealed aboard the house default flies her at the picket: it heaves to,
///    stands for customs, and locks nothing. This is the half the filed story compares with.
#[test]
fn mw13_the_verb_refuses_a_draft_and_a_coreless_hull_and_the_house_default_locks_nothing() {
    let mut rig = crate::excursions::stocked_fitted();
    let (pilot, row) = docked_dray(&mut rig, ("Pilot", 1, "Salt Moth"), (53, 19));
    let hull = ObjId(row.hull);

    // 1. No core.
    let none = run(&mut rig.tw, pilot, "+ship/mind");
    assert_eq!(none, ship_mind::MIND_NONE.replace("{name}", "Salt Moth"));
    let coreless = run(&mut rig.tw, pilot, "+ship/mind filed:1 go");
    assert_eq!(
        coreless,
        ship_mind::MIND_NO_CORE.replace("{name}", "Salt Moth")
    );

    // 2. A cored Dray is a Dray.
    crate::ic_sortie_surface::fit(&mut rig, hull, SHIP_II);
    let core = mush_tactics::module::unit_of(rig.tw.world(), hull)
        .and_then(tacsim::arena::core::read_core)
        .expect("she reads a core");
    assert_eq!(
        (core.class, core.governed),
        (tacsim::arena::core::CoreClass::II, true)
    );
    let status = run(&mut rig.tw, pilot, "+ship");
    assert!(status.contains("Salt Moth"), "{status}");
    let plan = run(&mut rig.tw, pilot, "+ship/plan stickney");
    assert!(
        plan.contains("Stickney"),
        "she plans as a Dray does: {plan}"
    );

    // 3. A draft.
    let draft = run(&mut rig.tw, pilot, "+ship/mind draft:Keeper go");
    assert_eq!(draft, tacsim::surface::dispatch::MIND_GOVERNED_DRAFT);
    let bare = run(&mut rig.tw, pilot, "+ship/mind Keeper");
    assert_eq!(bare, tacsim::surface::dispatch::MIND_GOVERNED_DRAFT);

    // 4. No claim path here.
    let unwired = run(&mut rig.tw, pilot, "+ship/mind filed:1 go");
    assert_eq!(unwired, ship_mind::MIND_UNWIRED);
    assert!(crate::excursions::drain(&mut rig.tw).is_empty());
    assert_eq!(cursor(&mut rig, hull), None);
    let usage = run(&mut rig.tw, pilot, "+ship/mind filed:1 org=1 extra");
    assert_eq!(usage, ship_mind::MIND_USAGE);

    // 5. The house default at the picket.
    to_the_picket_with_no_link(&mut rig, pilot, hull);
    let hove = until_at_the_picket(&mut rig, hull, 600, |now| {
        now.status != PicketInspectionStatus::AwaitingHeaveTo
    })
    .expect("she is still at the picket");
    assert_ne!(
        hove.status,
        PicketInspectionStatus::RanThePicket,
        "the house default heaves to: {hove:?}"
    );
    assert_eq!(hove.lock, None, "the house default locks nothing: {hove:?}");
    assert!(
        hove.trace
            .as_deref()
            .is_some_and(|word| word.starts_with("BACKUP_")),
        "the house default's own words: {hove:?}"
    );

    // And on the ungoverned twin a draft is refused as not built.
    let mut rig = crate::excursions::stocked_fitted();
    let (pilot, row) = docked_dray(&mut rig, ("Pilot", 1, "Salt Moth"), (53, 19));
    crate::ic_sortie_surface::fit(&mut rig, ObjId(row.hull), SHIP_II_UNGOVERNED);
    let draft = run(&mut rig.tw, pilot, "+ship/mind draft:Keeper go");
    assert_eq!(draft, ship_mind::MIND_DRAFT);
}

/// Print why the claim's owners refuse a transition (their `ic_claim` target says so at debug
/// and the player's sentence is generic), with the story's own log.
fn claim_log() -> tracing::subscriber::DefaultGuard {
    tracing::subscriber::set_default(
        tracing_subscriber::fmt()
            .with_ansi(false)
            .with_env_filter(tracing_subscriber::EnvFilter::new("ic_claim=debug,warn"))
            .with_writer(std::io::stderr)
            .finish(),
    )
}

/// The combined world with the pilot's chartered Dray docked at Ceres, the pilot aboard.
fn world() -> M8World {
    m8_world_manned(None, |rig| {
        let (pilot, row) = docked_dray(rig, ("Pilot", 1, "Salt Moth"), (53, 19));
        (pilot, row, Vec::new())
    })
}

/// **Seal `licence` aboard the pilot's hull and see the claim open at her berth**: the verb's
/// one submission landed, the cursor filed, and the port tick's arm opening the Roll's claim.
fn seal_aboard(world: &mut M8World, licence: u64) -> Aboard {
    seal_aboard_losing(world, licence, false)
}

/// [`seal_aboard`]; with `lose_the_first_open`, the arm's first submission of the Open is
/// dropped as a lost batch is, and the arm has to submit it again from the cursor.
fn seal_aboard_losing(world: &mut M8World, licence: u64, lose_the_first_open: bool) -> Aboard {
    let (pilot, hull) = (world.pilot, ObjId(world.row.hull));
    let (out, queued) = command(
        &mut world.rig,
        pilot,
        &format!("+ship/mind filed:{licence} go"),
    );
    assert_eq!(queued.len(), 1, "one submission: {out}");
    crate::excursions::land(&mut world.rig, &queued[0])
        .unwrap_or_else(|refused| panic!("+ship/mind: the go lands: {refused}"));
    assert!(
        matches!(
            cursor(&mut world.rig, hull),
            Some(ClaimCursor::Opening { operator, licence: l, .. }) if operator == pilot && l == licence
        ),
        "the cursor is filed: {:?}",
        cursor(&mut world.rig, hull)
    );
    if lose_the_first_open {
        let (_, queued) = crate::excursions::fire(&mut world.rig);
        assert!(
            queued
                .iter()
                .any(|submit| submit.action.key().id.as_str() == "open_ic_mind"),
            "the arm submitted the Open: {queued:?}"
        );
        drop(queued);
        assert_eq!(
            tacsim::ic_claim_of(world.rig.tw.world(), hull),
            tacsim::IcClaimRead::None,
            "the lost Open opened nothing"
        );
    }
    for _ in 0..12 {
        fires(world, 1);
        if let Some(aboard) = ship_mind::aboard(world.rig.tw.world(), hull) {
            return aboard;
        }
    }
    panic!(
        "the claim did not open at her berth: {:?}",
        cursor(&mut world.rig, hull)
    )
}

/// **Doc 44 unit MW.13 (Q-44-26): the done-when, on the production composition.**
///
/// 1. *Sealed, and a draft refused.* On her governed hull the pilot's unfiled draft is refused.
///    She has the Registry rate it and files it; `+ship/mind filed:<n> go` seals it: the
///    Roll opens the claim at her berth and the claim's own capture is the seal, with her as
///    its principal, at the class her core gives it, with the helm and the sensors and no fire
///    channel. A second seal is refused while the first stands.
/// 2. *A choice the house default does not make, inside a bubble.* She files for Stickney and
///    loses her link. At the Kirkwood picket her mind, not the house default, has her: it
///    locks the customs corvette, which no house default may, and heaves her to.
/// 3. *The same seal after a restart.* The server restarts with her link down and the mind
///    at her helm in the bubble. The seal reads the same class, principal and claim, and the
///    mind goes on flying her.
/// 4. *The claim closes once when she docks.* She makes Stickney; the passage's close moves
///    the cursor in its own bundle and the arm lands the Close. The Close's first submission
///    is lost with a restart, the arm owes it again from the record, and the book holds the
///    claim closed once, the licence's seat free.
#[tokio::test(flavor = "current_thread")]
#[ignore = "MW.13 story: one genuine Registry rating; the CI dispatch runs it"]
#[allow(clippy::too_many_lines)]
async fn mw13_a_filed_mind_sealed_aboard_flies_an_absent_pilots_ship_and_its_claim_closes_once() {
    let _log = claim_log();
    let mut world = world();
    let (pilot, hull) = (world.pilot, ObjId(world.row.hull));
    crate::ic_sortie_surface::fit(&mut world.rig, hull, SHIP_II);

    // 1. A draft is refused; the filed mind is sealed.
    let licence = crate::c4_filed_mind_field::file_own_mind(&mut world, "Keeper", &keeper()).await;
    let draft = run(&mut world.rig.tw, pilot, "+ship/mind draft:Keeper go");
    assert_eq!(draft, tacsim::surface::dispatch::MIND_GOVERNED_DRAFT);
    assert_eq!(
        cursor(&mut world.rig, hull),
        None,
        "a refusal files nothing"
    );
    let preview = run(
        &mut world.rig.tw,
        pilot,
        &format!("+ship/mind filed:{licence}"),
    );
    assert_eq!(
        preview,
        ship_mind::MIND_PREVIEW
            .replace("{licence}", &licence.to_string())
            .replace("{name}", "Salt Moth")
    );
    // The arm's first Open is lost, as a batch is; it submits the Open again from the cursor.
    let sealed = seal_aboard_losing(&mut world, licence, true);
    eprintln!("MW13 sealed {sealed:?}");
    assert_eq!(sealed.dispatcher, pilot);
    assert_eq!(sealed.principal, Some(pilot), "she is the seal's principal");
    // The Registry files a mind under its name in lower case.
    assert_eq!(sealed.mind.as_str(), "keeper");
    assert_eq!(
        (sealed.core_class, sealed.governed),
        (tacsim::arena::core::CoreClass::II, true),
        "the seal is over her governed Class II core"
    );
    assert!(sealed.class <= sealed.core_class);
    assert_eq!(
        sealed.channels,
        vec![
            tacsim::arena::state::Channel::Helm,
            tacsim::arena::state::Channel::Sensors
        ],
        "helm and sensors, and no fire channel"
    );
    assert_eq!(
        tacsim::ic_claim_of(world.rig.tw.world(), hull),
        tacsim::IcClaimRead::Active {
            open: sealed.claim,
            dispatcher: pilot
        }
    );
    let read = run(&mut world.rig.tw, pilot, "+ship/mind");
    assert_eq!(
        read,
        ship_mind::MIND_ABOARD
            .replace("{mind}", "keeper")
            .replace("{name}", "Salt Moth")
            .replace("{licence}", &licence.to_string())
            .replace("{class}", sealed.class.as_str())
    );
    let again = run(
        &mut world.rig.tw,
        pilot,
        &format!("+ship/mind filed:{licence} go"),
    );
    assert_eq!(
        again,
        ship_mind::MIND_ALREADY.replace("{name}", "Salt Moth")
    );

    // A restart at her berth, the claim open and she not yet sailed: the same seal.
    let mut world = world.reboot();
    assert_eq!(
        ship_mind::aboard(world.rig.tw.world(), hull).as_ref(),
        Some(&sealed),
        "a restart at her berth keeps the seal"
    );
    fires(&mut world, 3);
    assert_eq!(
        tacsim::ic_claim_of(world.rig.tw.world(), hull),
        tacsim::IcClaimRead::Active {
            open: sealed.claim,
            dispatcher: pilot
        },
        "and opens no second claim"
    );

    // 2. At the picket, with her link down, her own mind has her. She sails, loses her link
    // on the corridor, and the server restarts with her under way: the boot admits a held
    // ship on her passage, and the seal is the same.
    let boundary = sail_and_lose_the_link(&mut world.rig, pilot, hull);
    eprintln!(
        "MW13 under way: ic {} of boundary {boundary}, rate {}",
        crate::excursions::ic_now(&world.rig.tw),
        world.rig.tw.world().world_timeline().current().rate
    );
    let mut world = world.reboot_reconnecting(&[mush_core::GOD]);
    eprintln!(
        "MW13 after the restart: ic {} of boundary {boundary}, rate {}, journey {:?}, linkdead {:?}",
        crate::excursions::ic_now(&world.rig.tw),
        world.rig.tw.world().world_timeline().current().rate,
        world
            .rig
            .tw
            .world()
            .module_state::<mush_sim::JourneyBook>()
            .and_then(|book| book.get(hull))
            .map(|journey| (journey.state().clone(), journey.leg_due_at(0))),
        world
            .rig
            .tw
            .world()
            .module_state::<SpaceBubbleBook>()
            .and_then(|book| book.linkdead_state(hull))
            .map(|state| (state.backup_has_helm(), state.backup_act)),
    );
    assert_eq!(
        ship_mind::aboard(world.rig.tw.world(), hull).as_ref(),
        Some(&sealed),
        "a restart under way keeps the seal"
    );
    assert!(
        matches!(
            tacsim::port::ship::berth_of(world.rig.tw.world(), hull),
            tacsim::port::ship::Berth::UnderWay
        ),
        "she is still under way"
    );
    arrive_at_the_picket(&mut world.rig, hull, boundary);
    let locked = until_at_the_picket(&mut world.rig, hull, 30, |now| now.locks_the_corvette)
        .expect("she is at the picket");
    assert!(
        locked.locks_the_corvette,
        "her mind locks the corvette that hails her, which no house default may: {locked:?}"
    );
    assert!(
        locked
            .trace
            .as_deref()
            .is_some_and(|word| word.starts_with("KEEPER_")),
        "her own mind answered, by its own words: {locked:?}"
    );
    assert_ne!(locked.thrust, [0, 0, 0], "and it heaves her to: {locked:?}");

    // 3. A restart with her link down: the same seal, and the mind flies on.
    let before = ship_mind::aboard(world.rig.tw.world(), hull).expect("the seal stands");
    assert_eq!(
        before, sealed,
        "the seal is the claim's, unchanged since her berth"
    );
    let mut world = world.reboot_reconnecting(&[mush_core::GOD]);
    let after = ship_mind::aboard(world.rig.tw.world(), hull).expect("the seal after a restart");
    assert_eq!(
        after, before,
        "the same class, principal and claim after the restart"
    );
    assert!(
        at_the_picket(&world.rig, hull).is_some(),
        "the bubble is read back"
    );
    let hove = until_at_the_picket(&mut world.rig, hull, 900, |now| {
        now.status != PicketInspectionStatus::AwaitingHeaveTo
    })
    .expect("she is still at the picket");
    assert_ne!(
        hove.status,
        PicketInspectionStatus::RanThePicket,
        "{hove:?}"
    );
    assert!(
        hove.trace
            .as_deref()
            .is_some_and(|word| word.starts_with("KEEPER_")),
        "the resumed mind is hers: {hove:?}"
    );
    assert!(hove.locks_the_corvette, "and her lock stands: {hove:?}");

    // 4. She makes Stickney, and the claim closes once.
    let stickney = crate::excursions::port_map(&world.rig.tw, "stickney_port")
        .host
        .map(ObjId)
        .expect("Stickney's host");
    let mut docked = false;
    for _ in 0..4_000 {
        if tacsim::port::ship::berth_of(world.rig.tw.world(), hull)
            .hangar()
            .is_some()
        {
            docked = true;
            break;
        }
        if at_the_picket(&world.rig, hull).is_some() {
            one_second(&mut world.rig);
            continue;
        }
        // Out of the bubble: stride to her journey's next leg end.
        let due = world
            .rig
            .tw
            .world()
            .module_state::<mush_sim::JourneyBook>()
            .and_then(|book| book.get(hull))
            .and_then(|journey| {
                (0..journey.route().legs().len()).find_map(|leg| {
                    journey
                        .leg_due_at(leg)
                        .filter(|due| *due > crate::excursions::ic_now(&world.rig.tw))
                })
            });
        match due {
            Some(due) => {
                let _ = crate::excursions::to_ic(&mut world.rig, due);
            }
            None => one_second(&mut world.rig),
        }
    }
    assert!(
        docked,
        "she makes port: {:?}, {:?}",
        at_the_picket(&world.rig, hull),
        world
            .rig
            .tw
            .world()
            .module_state::<SpaceBubbleBook>()
            .and_then(|book| book.linkdead_state(hull))
            .cloned()
    );
    assert!(
        matches!(
            tacsim::port::ship::berth_of(world.rig.tw.world(), hull),
            tacsim::port::ship::Berth::Docked { map, .. } if map.host == Some(stickney.0)
        ),
        "at Stickney"
    );
    // The passage's close moves the cursor in its own bundle.
    let mut closing = None;
    for _ in 0..12 {
        if let Some(ClaimCursor::Closing { open, kind, .. }) = cursor(&mut world.rig, hull) {
            assert_eq!(
                kind,
                tacsim::surface::excursion::ClaimCloseKind::ReturnedToBase { base: stickney },
                "the claim closes by the kind her passage closed by, in the epoch's own bundle"
            );
            closing = Some(open);
            break;
        }
        // One fire, its submissions landed but for a claim's Close, which is lost.
        let (_, queued) = crate::excursions::fire(&mut world.rig);
        crate::excursions::land_all(&mut world.rig, &queued);
    }
    assert_eq!(
        closing,
        Some(sealed.claim),
        "the epoch's close owes the claim's Close: {:?}",
        cursor(&mut world.rig, hull)
    );
    assert!(
        ship_mind::aboard(world.rig.tw.world(), hull).is_some(),
        "the book still holds the claim open until its Close lands"
    );
    // The Close's first submission is lost, with a restart.
    let (_, queued) = crate::excursions::fire(&mut world.rig);
    assert!(
        !queued.is_empty(),
        "the arm submitted the Close: {:?}",
        cursor(&mut world.rig, hull)
    );
    drop(queued);
    assert!(tacsim::ic_claim_closed(world.rig.tw.world(), sealed.claim).is_none());
    let mut world = world.reboot_reconnecting(&[mush_core::GOD]);
    assert!(
        matches!(
            cursor(&mut world.rig, hull),
            Some(ClaimCursor::Closing { open, .. }) if open == sealed.claim
        ),
        "the record owes the Close again"
    );
    let mut closed = None;
    for _ in 0..12 {
        fires(&mut world, 1);
        closed = tacsim::ic_claim_closed(world.rig.tw.world(), sealed.claim);
        if closed.is_some() {
            break;
        }
    }
    let closed = closed.expect("the claim's Close lands");
    assert_eq!(
        tacsim::ic_claim_of(world.rig.tw.world(), hull),
        tacsim::IcClaimRead::None,
        "no claim stands on her"
    );
    assert_eq!(ship_mind::aboard(world.rig.tw.world(), hull), None);
    // Once: more fires and a restart close nothing more and open nothing.
    fires(&mut world, 6);
    let mut world = world.reboot_reconnecting(&[mush_core::GOD]);
    fires(&mut world, 6);
    assert_eq!(
        tacsim::ic_claim_closed(world.rig.tw.world(), sealed.claim),
        Some(closed),
        "the claim closed once"
    );
    assert_eq!(
        tacsim::ic_claim_of(world.rig.tw.world(), hull),
        tacsim::IcClaimRead::None
    );
    assert!(
        matches!(
            cursor(&mut world.rig, hull),
            Some(ClaimCursor::Closing { open, .. }) if open == sealed.claim
        ),
        "the cursor rests on the closed claim"
    );
}

/// Fire the port tick until the book holds `claim` closed, landing what each fire queues;
/// what the pilot heard meanwhile.
fn until_closed(world: &mut M8World, claim: mush_core::CommitToken) -> String {
    let pilot = world.pilot;
    let mut heard = String::new();
    let mut closed_at = None;
    for fire in 0..20 {
        let (events, queued) = crate::excursions::fire(&mut world.rig);
        heard.push_str(&crate::excursions::said(&world.rig.tw, &events, pilot));
        heard.push('\n');
        crate::excursions::land_all(&mut world.rig, &queued);
        world.pulse();
        if closed_at.is_none() && tacsim::ic_claim_closed(world.rig.tw.world(), claim).is_some() {
            closed_at = Some(fire);
        }
        // One fire more, for the arm to acknowledge the landed Close.
        if closed_at.is_some_and(|at| fire > at) {
            break;
        }
    }
    assert!(
        closed_at.is_some(),
        "the claim did not close; the pilot heard: {heard}"
    );
    heard
}

/// **Doc 44 unit MW.13 (second review, row 25): a seal does not outlive its core or its
/// holder.** One filed mind, sealed twice on the pilot's Dray at her berth.
///
/// A. *Her fit cannot change under a sealed mind.* With the claim open the Co-op's yard
///    refuses to re-dress her, by a sentence that says to unseal first, and the unit's owner
///    refuses staff's rebind; her core and her seal are as they were. `+ship/mind off go`
///    unseals her and the claim closes. Then the yard re-dresses her as a Dray-P, which
///    carries no core, and nothing of the old seal is left to seal again: a new `+ship/mind`
///    is resolved against the hull as she now is, and is refused for want of a core.
/// B. *A change of title.* Sealed again, her title's claims are released and she is titled
///    to a House the pilot holds no office with, as a sale or a sweep titles a hull to its new
///    holder. No owner of the claim sees that. The seat reads no mind aboard at once, the
///    pilot is told why, the arm closes the claim, and no claim stands on the hull for the new
///    holder to inherit. The pilot can no longer seal a mind on her.
#[tokio::test(flavor = "current_thread")]
#[ignore = "MW.13 story: one genuine Registry rating; the CI dispatch runs it"]
#[allow(clippy::too_many_lines)]
async fn mw13_a_seal_does_not_outlive_its_core_or_its_holder() {
    let _log = claim_log();
    // No provisions aboard: the yard refits a hull whose hold is empty but for her tank.
    let mut world = m8_world_manned(None, |rig| {
        let (pilot, row) = docked_dray(rig, ("Pilot", 1, "Salt Moth"), (20, 0));
        (pilot, row, Vec::new())
    });
    let (pilot, hull) = (world.pilot, ObjId(world.row.hull));
    crate::ic_sortie_surface::fit(&mut world.rig, hull, SHIP_II);
    let licence = crate::c4_filed_mind_field::file_own_mind(&mut world, "Keeper", &keeper()).await;
    let seal_line = format!("+ship/mind filed:{licence} go");
    let core_of = |world: &M8World| {
        mush_tactics::module::unit_of(world.rig.tw.world(), hull)
            .and_then(tacsim::arena::core::read_core)
            .map(|core| (core.class, core.governed))
    };
    let class_two = Some((tacsim::arena::core::CoreClass::II, true));

    // A. Her fit cannot change under a sealed mind.
    let first = seal_aboard(&mut world, licence);
    // She goes ashore to the Co-op's desk: out through her lock, as a pilot does.
    let ashore = run(&mut world.rig.tw, pilot, "+ship/leave");
    assert!(
        tacsim::space::hull_aboard_for_actor(world.rig.tw.world(), pilot) != Some(hull),
        "she is ashore: {ashore}"
    );
    let desk = crate::excursions::station_room(&world.rig.tw, "ceres_port", "room.c05_registry");
    crate::excursions::teleport(&mut world.rig.tw, pilot, desk);
    let held = ship_mind::MIND_HOLDS_HER.replace("{name}", "Salt Moth");
    assert_eq!(
        run(&mut world.rig.tw, pilot, "+coop/refit prospector"),
        held,
        "the yard says to unseal first"
    );
    let removed = run(
        &mut world.rig.tw,
        mush_core::GOD,
        &format!("@unit/remove #{}", hull.0),
    );
    assert!(
        removed.contains("held by an IC sortie"),
        "the unit's owner holds her as she is: {removed}"
    );
    assert_eq!(core_of(&world), class_two, "her core is as it was");
    assert_eq!(
        ship_mind::aboard(world.rig.tw.world(), hull).as_ref(),
        Some(&first),
        "and so is her seal"
    );
    // Unsealed at her berth, the claim closes.
    assert_eq!(
        run(&mut world.rig.tw, pilot, "+ship/mind off"),
        ship_mind::MIND_OFF_PREVIEW.replace("{name}", "Salt Moth")
    );
    let (out, queued) = command(&mut world.rig, pilot, "+ship/mind off go");
    assert_eq!(queued.len(), 1, "one submission: {out}");
    crate::excursions::land(&mut world.rig, &queued[0])
        .unwrap_or_else(|refused| panic!("+ship/mind off: the go lands: {refused}"));
    assert!(
        matches!(
            cursor(&mut world.rig, hull),
            Some(ClaimCursor::Closing { open, kind: tacsim::surface::excursion::ClaimCloseKind::PoweredDownInBase { .. }, .. })
                if open == first.claim
        ),
        "{:?}",
        cursor(&mut world.rig, hull)
    );
    let heard = until_closed(&mut world, first.claim);
    assert!(
        heard.contains(&ship_mind::MIND_UNSEALED.replace("{name}", "Salt Moth")),
        "{heard}"
    );
    assert_eq!(ship_mind::aboard(world.rig.tw.world(), hull), None);
    assert_eq!(
        run(&mut world.rig.tw, pilot, "+ship/mind"),
        ship_mind::MIND_NONE.replace("{name}", "Salt Moth")
    );
    // Now the yard re-dresses her, and nothing of the old seal seals again.
    let refit = run(&mut world.rig.tw, pilot, "+coop/refit prospector");
    assert!(refit.contains("as a Dray-P"), "{refit}");
    assert_eq!(core_of(&world), None, "the Dray-P carries no core");
    assert_eq!(
        run(&mut world.rig.tw, pilot, &seal_line),
        ship_mind::MIND_NO_CORE.replace("{name}", "Salt Moth"),
        "a new seal is asked of the hull as she is"
    );
    fires(&mut world, 4);
    assert_eq!(
        tacsim::ic_claim_of(world.rig.tw.world(), hull),
        tacsim::IcClaimRead::None,
        "and no claim opens on her"
    );
    let back = run(&mut world.rig.tw, pilot, "+coop/refit standard");
    assert!(back.contains("as a standard Dray"), "{back}");
    crate::ic_sortie_surface::fit(&mut world.rig, hull, SHIP_II);

    // B. Her title passes to a holder the pilot does not act for.
    let second = seal_aboard(&mut world, licence);
    assert_ne!(second.claim, first.claim, "a new seal is a new claim");
    crate::m8_world::title_rig_to_house(&mut world, hull, "Meridiani");
    assert_eq!(
        ship_mind::aboard(world.rig.tw.world(), hull),
        None,
        "a hull under a new title carries no seal of her last holder's"
    );
    assert_eq!(
        run(&mut world.rig.tw, pilot, "+ship/mind"),
        ship_mind::MIND_UNFIT
            .replace("{mind}", "keeper")
            .replace("{name}", "Salt Moth")
    );
    let heard = until_closed(&mut world, second.claim);
    assert!(
        heard.contains(
            &ship_mind::MIND_VOIDED
                .replace("{name}", "Salt Moth")
                .replace("{why}", "she is no longer yours to fly it on")
        ),
        "{heard}"
    );
    assert_eq!(
        tacsim::ic_claim_of(world.rig.tw.world(), hull),
        tacsim::IcClaimRead::None,
        "no claim stands on the hull for her new holder to inherit"
    );
    let again = run(&mut world.rig.tw, pilot, &seal_line);
    assert!(
        again.contains("a licensed mind flies a hull titled to you"),
        "the pilot seals nothing on a hull that is another's: {again}"
    );
    // Each claim closed, and a restart opens none again.
    let mut world = world.reboot();
    fires(&mut world, 4);
    for claim in [first.claim, second.claim] {
        assert!(tacsim::ic_claim_closed(world.rig.tw.world(), claim).is_some());
    }
    assert_eq!(
        tacsim::ic_claim_of(world.rig.tw.world(), hull),
        tacsim::IcClaimRead::None
    );
    assert_eq!(ship_mind::aboard(world.rig.tw.world(), hull), None);
}

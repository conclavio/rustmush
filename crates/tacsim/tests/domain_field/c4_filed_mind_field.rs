//! **Doc 44 unit MW.5: a player's filed mind flies an absent player's unit in a field, on the
//! production composition** (the story doc 44 unit MW.7 left: Q-44-38, Q-44-46; doc 17g C4).
//!
//! MW.7 built the absent player's rule and pinned it for a sealed script at the arena, where a
//! script can be sealed without the Registry. Its field stories run on the driving rig, which
//! composes no launch, and fly the house mind. Here the world is the combined M8 world (the
//! production `LaunchComposition`, `m8_world`), and the mind is one its pilot wrote, had the
//! Registry rate, filed, and launched under their own owner's licence, so the unit is admitted
//! to the field under the seal of a claim the Roll opened in the bay.
//!
//! The unit is play's own from end to end. The pilot leases Occator's escort at Ceres
//! (`+tv/lease escort go`), takes the operational loan, sails to Ahuna Flank, writes the script
//! with the workshop's verbs, rates it (`@mind/rate`), files it (`@mind/file`), boards the rig
//! in the bay and launches it with `mind=filed:<licence>`. MW.22 admits the filed mind on the
//! leased hull by its lease. The two other parties are the field stories' usual stand-ins: a
//! fixture hull each, a second and a third captain seated in them by staff (CF.2's `armed` and
//! `seat`), and the raider's captain types every shot.
//!
//! Two entries share the script:
//!
//! - [`c4_the_house_mind_takes_an_absent_pilots_escort_home_on_the_launch`] flies no filed mind
//!   and takes no rating, so it runs in the default sweep: MW.7's rule over the launch
//!   composition, for the mind every hull has.
//! - [`c4_a_filed_mind_flies_an_absent_pilots_escort_and_is_taken_home`] is the owed story. It
//!   takes one genuine Registry rating, so the CI dispatch runs it.

use mush_core::ObjId;
use tacsim::arena::ledger::{EventKind, LedgerEvent};
use tacsim::arena::party::Relation;
use tacsim::arena_task::ArenaOut;
use tacsim::field::absent::{away_of, minded};
use tokio::sync::mpsc::Receiver;

use crate::escort_lease::{fired, in_field};
use crate::excursions::run;
use crate::field_host_rig::{armed, egress, frame, seat, task, until_composed, weapon_of};
use crate::m8_world::{M8World, claimed_launch, fires, into_field_rig_kept, m8_world_manned};

/// Ahuna Flank.
const SITE: (&str, &str) = ("ahuna", "ahuna_port");
/// Where the pilot's escort stands, the raider eight cells south of it, and the third captain's
/// hull (`field_mw7`'s yard).
const WARD_AT: (i64, i64) = (146, 114);
const RAIDER_AT: (i64, i64) = (146, 122);
const THIRD_AT: (i64, i64) = (136, 122);

/// How long the script stands and asks before it goes home, in milliseconds of the field's own
/// clock.
const STANDS_MS: i64 = 40_000;

/// **The pilot's own script, written to fire first.** A cold hull it starts up (the Registry's
/// trials begin cold). Then at every think it picks a contact that is not a
/// friend and does not read dead (one that reads hostile, where there is one), and, while its
/// cannon holds a round, arms the cannon on a lock, locks the contact and fires. It asks
/// whether or not anyone has fired on it. Told to go home, it stands and keeps asking for
/// forty seconds, then drives for its party's entry, which is its carrier's ramp.
///
/// The two guards are what the Registry asks of any script: an intent the hull cannot carry out
/// (a shot from a cannon that is spent, a lock on a wreck) is a fault, and one fault in one
/// think of one trial ends the rating as failed.
fn fires_first() -> String {
    format!(
        "fn think(v, c) {{ \
           if v.power == \"off\" {{ startup(); return; }} \
           if v.power != \"online\" {{ return; }} \
           let gun = (); \
           for w in v.weapons {{ if w.id == \"cannon\" && w.rounds > 0 {{ gun = w.id; }} }} \
           let id = (); \
           for k in v.contacts {{ if !k.friendly && k.reads_dead != true {{ id = k.id; }} }} \
           for k in v.contacts {{ if k.relation == \"hostile\" && k.reads_dead != true {{ id = k.id; }} }} \
           if id != () && gun != () {{ arm(gun, \"lock\", 100, 15, 0, 0, 4294967295); lock(id); fire(gun, 1); }} \
           if v.order != () && v.order.objective == \"withdraw\" {{ \
             if !(\"since\" in this) {{ this.since = v.t_ms; }} \
             if v.t_ms - this.since >= {STANDS_MS} {{ let home = entry_cell(v); if home != () {{ goto(home); }} }} \
           }} \
         }}"
    )
}

/// **Lease the escort at Occator's counter and take the operational loan** (`escort_lease`'s
/// `lease_escort`, on the excursions rig): the signature, then the loan's Org submissions landed
/// on the composed path.
fn lease_escort(rig: &mut crate::excursions::Rig, pilot: ObjId) {
    let counter = crate::excursions::station_room(&rig.tw, "ceres_port", "room.c07_exchange");
    crate::excursions::teleport(&mut rig.tw, pilot, counter);
    let signed = run(&mut rig.tw, pilot, "+tv/lease escort go");
    assert!(
        signed.contains("Occator Water & Propellant leases you OWP-ES-01"),
        "{signed}"
    );
    let loan = run(&mut rig.tw, pilot, "+tv/1/permissions go");
    assert!(
        loan.contains("recording the four operational loans"),
        "{loan}"
    );
    let mut queued = Vec::new();
    while let Some(submit) = rig.tw.engine_mut().take_authoritative_submit() {
        queued.push(*submit);
    }
    let grants: Vec<_> = queued
        .into_iter()
        .filter(|submit| submit.action.key().id.as_str() == "unitgrant_issue")
        .collect();
    assert!(!grants.is_empty(), "the loan is submitted");
    for grant in &grants {
        crate::excursions::land(rig, grant).expect("the loan lands");
    }
}

/// **The combined world with the escort leased and two more captains on the pad**: the pilot's
/// Dray-P on Ahuna Flank with the leased escort in its bay, and a second and a third captain's
/// hulls beside it, before the launch's identities are made and the launch boots.
fn world() -> (M8World, ObjId, ObjId) {
    let mut others = None;
    let world = m8_world_manned(None, |rig| {
        let party = crate::excursions::on_pad_leasing_at(
            rig,
            ("Pilot", 1, "Salt Moth"),
            SITE,
            lease_escort,
        );
        let (captain, _, _) =
            crate::excursions::on_pad_as_at(rig, ("Other", 2, "Tally Moth"), SITE, 0);
        let (stranger, _, _) =
            crate::excursions::on_pad_as_at(rig, ("Third", 3, "Lone Moth"), SITE, 0);
        others = Some((captain, stranger));
        party
    });
    assert_eq!(world.tvs.len(), 1, "the escort is the hull's one rig");
    let (captain, stranger) = others.expect("the other captains");
    (world, captain, stranger)
}

/// **The pilot writes, saves, rates and files their own mind**; the owner's licence the filing
/// issued them. The workshop's verbs write the draft (`@mind/new`, `@mind/line`, `@mind/save`),
/// the Registry rates the saved revision through its real trials (`@mind/rate`), and the pilot
/// files it from their own wallet (`@mind/file`, twice: the charter's shell, then the filing).
async fn file_own_mind(world: &mut M8World, name: &str, source: &str) -> u64 {
    use std::time::{Duration, Instant};
    let pilot = world.pilot;
    crate::m8_end_user::fund(world, pilot, 100_000);
    let said = run(&mut world.rig.tw, pilot, &format!("@mind/new {name}"));
    assert!(said.contains("Mind draft created"), "{said}");
    let lines = world
        .rig
        .tw
        .world()
        .module_state::<mush_roll::RollStore>()
        .and_then(|store| store.mind(pilot, name).ok())
        .map(|mind| mind.draft().source.lines().count())
        .expect("the draft");
    let said = run(
        &mut world.rig.tw,
        pilot,
        &format!("@mind/line {name} 1={source}"),
    );
    assert!(said.contains("Working source updated"), "{said}");
    if lines > 1 {
        let said = run(
            &mut world.rig.tw,
            pilot,
            &format!("@mind/delete {name} 2-{lines}"),
        );
        assert!(said.contains("Working source updated"), "{said}");
    }
    let said = run(
        &mut world.rig.tw,
        pilot,
        &format!("@mind/save {name}=first"),
    );
    eprintln!("C4_FIELD save -> {said}");
    let saved = |world: &M8World| -> Option<mush_roll::Revision> {
        let store = world
            .rig
            .tw
            .world()
            .module_state::<mush_roll::RollStore>()?;
        let mind = store.mind(pilot, name).ok()?;
        mind.latest()
            .and_then(|reference| store.revision(pilot, reference).ok())
            .cloned()
    };
    let deadline = Instant::now() + Duration::from_secs(120);
    let revision = loop {
        world.pulse();
        if let Some(revision) = saved(world) {
            break revision;
        }
        assert!(
            Instant::now() < deadline,
            "the author service did not save {name}"
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    };
    let rated = run(&mut world.rig.tw, pilot, &format!("@mind/rate {name}@1"));
    assert!(rated.contains("Registry preparation queued"), "{rated}");
    let identity =
        mush_roll::commerce::RevisionIdentity::capture(&revision).expect("the revision identity");
    let started = Instant::now();
    let deadline = started + Duration::from_secs(7_200);
    let mut told = Instant::now();
    loop {
        world.pulse();
        let done = world.rows("rating_receipts").into_iter().any(|wire| {
            serde_json::from_value::<mush_roll::commerce::rating::RatingReceipt>(wire)
                .is_ok_and(|receipt| receipt.subject().revision() == identity)
        });
        if done {
            break;
        }
        // The Registry's own job for this revision: queued or running is a wait; any other
        // end without a receipt (nonconforming, failed, interrupted, cancelled) is the story's
        // failure, said at once and not at the guard.
        use mush_roll::commerce::rating::{RatingJob, RatingJobState};
        let jobs: Vec<RatingJob> = world
            .rows("rating_jobs")
            .into_iter()
            .filter_map(|row| serde_json::from_value(row).ok())
            .filter(|job: &RatingJob| job.subject().revision() == identity)
            .collect();
        for job in &jobs {
            match job.state() {
                RatingJobState::Queued {}
                | RatingJobState::Running { .. }
                | RatingJobState::Completed { .. } => {}
                other => panic!("the Registry ended {name}'s rating without a receipt: {other:?}"),
            }
        }
        if told.elapsed() >= Duration::from_secs(120) {
            told = Instant::now();
            eprintln!(
                "C4_FIELD rating {name}: {} s, jobs {:?}",
                started.elapsed().as_secs(),
                jobs.iter().map(RatingJob::state).collect::<Vec<_>>()
            );
        }
        assert!(
            Instant::now() < deadline,
            "the Registry did not rate {name}"
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    eprintln!("C4_FIELD rated {name} in {} s", started.elapsed().as_secs());
    // The filing. `@mind/file` first reserves the charter's shell, and once that has landed the
    // same line files the rated revision; each is the pilot's own submission, landed on the
    // composed path. The Registry releases its job a pulse after the receipt, so the line is
    // typed again until the filing itself has been staged and landed.
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let said = run(&mut world.rig.tw, pilot, &format!("@mind/file {name}@1"));
        eprintln!("C4_FIELD file -> {said}");
        let mut staged = Vec::new();
        while let Some(submit) = world.rig.tw.engine_mut().take_authoritative_submit() {
            staged.push(*submit);
        }
        for submit in &staged {
            crate::excursions::land(&mut world.rig, submit)
                .unwrap_or_else(|refused| panic!("@mind/file: {refused}"));
        }
        if said.contains("Filing ") && !staged.is_empty() {
            break;
        }
        assert!(Instant::now() < deadline, "the filing never landed: {said}");
        world.pulse();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let row = world
        .rows("roll_mind_initial_filing")
        .into_iter()
        .last()
        .expect("the pilot's filing");
    let licence = row
        .get("licence")
        .and_then(|licence| {
            licence
                .as_u64()
                .or_else(|| licence.as_str().and_then(|text| text.parse().ok()))
        })
        .unwrap_or_else(|| panic!("the filing names its owner's licence: {row}"));
    eprintln!("C4_FIELD filed {name}: owner's licence {licence}");
    licence
}

/// **Board the escort in the bay and take its controls** (`escort_lease`'s `board_escort`, on
/// the excursions rig): the hold, the bay, the cab, the focus and the handoff.
fn board(world: &mut M8World, tv: ObjId) {
    let pilot = world.pilot;
    for step in ["inner", "hold"] {
        let _ = run(&mut world.rig.tw, pilot, step);
    }
    let bay = run(&mut world.rig.tw, pilot, "+tv/bay in");
    assert!(bay.contains("entered vehicle bay"), "{bay}");
    let boarded = run(&mut world.rig.tw, pilot, "+tv/1 board");
    assert!(boarded.contains("You are at rig 1's"), "{boarded}");
    let focus = run(&mut world.rig.tw, pilot, &format!("+focus #{}/pilot", tv.0));
    assert!(!focus.contains("Usage"), "{focus}");
    let _ = run(
        &mut world.rig.tw,
        pilot,
        &format!("@station/handoff #{}/pilot", tv.0),
    );
    fires(world, 3);
}

/// Every connection of `who` dropped as a clean QUIT.
fn quit(rig: &mut crate::driving_rig::Rig, who: ObjId) {
    let mut out = mush_core::Outbox::new();
    let mut fx = mush_core::Effects::new();
    let conns: Vec<u64> = (0..=64)
        .filter(|conn| rig.tw.engine().actor_of(*conn) == Some(who))
        .collect();
    assert!(!conns.is_empty(), "{who} holds a connection");
    for conn in conns {
        rig.tw.engine_mut().step(
            mush_core::Inbound::ConnDown {
                conn,
                actor: who,
                reason: mush_core::DisconnectKind::Quit,
            },
            &mut out,
            &mut fx,
        );
    }
}

/// The rounds `shooter` fired at `target`.
fn fired_at(lines: &[LedgerEvent], shooter: ObjId, target: ObjId) -> usize {
    lines
        .iter()
        .filter(|line| {
            matches!(&line.kind, EventKind::Fired { unit, target: at, .. }
                if *unit == shooter && *at == target)
        })
        .count()
}

/// The rounds of `shooter` that arrived on `target`, hit or miss.
fn arrived(lines: &[LedgerEvent], shooter: ObjId, target: ObjId) -> usize {
    lines
        .iter()
        .filter(|line| {
            matches!(&line.kind, EventKind::Impact { unit, target: at, .. }
                if *unit == shooter && *at == target)
        })
        .count()
}

/// The slots `unit` armed and the locks it took.
fn asked(lines: &[LedgerEvent], unit: ObjId) -> usize {
    lines
        .iter()
        .filter(|line| {
            matches!(&line.kind, EventKind::Armed { unit: by, .. } | EventKind::Locked { unit: by, .. }
                if *by == unit)
        })
        .count()
}

/// Where the black box last put `unit`, as a site cell.
fn last_cell(lines: &[LedgerEvent], unit: ObjId) -> Option<(i64, i64)> {
    lines.iter().rev().find_map(|line| match &line.kind {
        EventKind::PositionSummary { units } => units
            .iter()
            .find(|row| row.0 == unit)
            .map(|row| tacsim::surface::excursion::site_cell(row.1, row.2)),
        _ => None,
    })
}

/// Whether the arena's registry holds a seat for `who`.
fn bound(rig: &crate::driving_rig::Rig, who: ObjId) -> bool {
    rig.tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .is_some_and(|registry| registry.seat_of_pilot(who).is_some())
}

/// **The raider's captain fires on the escort for `secs` seconds** (the story's step 5): a
/// lock kept on the escort, the contact on their own frame nearest where it stands, and a shot
/// a wall second. What came of it is read from the black box at the story's end
/// ([`answered_in_order`]): the box reaches the host in batches, so nothing here waits on it.
async fn raided(
    rig: &mut crate::driving_rig::Rig,
    rx: &mut Receiver<ArenaOut>,
    lines: &mut Vec<LedgerEvent>,
    (raider, stranger): (ObjId, ObjId),
    secs: u32,
) {
    let gun = weapon_of(rig, raider);
    for _ in 0..secs {
        let escort = frame(rig, stranger).and_then(|frame| {
            let lock = frame.lock;
            frame
                .contacts
                .iter()
                .filter_map(|contact| {
                    let (x, y) = contact.cell?;
                    let off = i64::from(x).abs_diff(WARD_AT.0) + i64::from(y).abs_diff(WARD_AT.1);
                    (off <= 6).then_some((off, contact.id, contact.designator.clone()))
                })
                .min_by_key(|(off, _, _)| *off)
                .map(|(_, id, designator)| (id, designator, lock))
        });
        if let Some((id, designator, lock)) = escort
            && lock != Some(id)
        {
            let _ = crate::driving_rig::run(&mut rig.tw, stranger, &format!("LOCK {designator}"));
        }
        let _ = crate::driving_rig::run(&mut rig.tw, stranger, &format!("FIRE {gun}"));
        let _ = until_composed(rig, rx, lines, 1, |_, _| false).await;
    }
    let _ = crate::driving_rig::run(&mut rig.tw, stranger, &format!("FIRE {gun} OFF"));
}

/// **What the black box holds of the exchange, in order** (the story's steps 4 and 5): the
/// raider's fire arrived on the escort; the escort fired at the raider, and its first round
/// left after that first arrival; and, written to fire first, the script had asked before
/// anyone fired on it. The pilot never armed a slot or took a lock, so an arming or a lock of
/// this unit ahead of the raider's first arrival is the script's own.
fn answered_in_order(lines: &[LedgerEvent], (tv, raider, third): (ObjId, ObjId, ObjId)) {
    let exchange = format!(
        "escort fired {} (at the raider {}, at the third {}); raider fired {}, arrived {}",
        fired(lines, tv),
        fired_at(lines, tv, raider),
        fired_at(lines, tv, third),
        fired(lines, raider),
        arrived(lines, raider, tv),
    );
    let first_arrival = lines
        .iter()
        .position(|line| {
            matches!(&line.kind, EventKind::Impact { unit, target, .. }
                if *unit == raider && *target == tv)
        })
        .unwrap_or_else(|| panic!("the raider's fire arrived on the escort: {exchange}"));
    assert!(
        fired_at(lines, tv, raider) > 0,
        "it answers the unit that fired on it: {exchange}"
    );
    let first_answer = lines
        .iter()
        .position(|line| matches!(&line.kind, EventKind::Fired { unit, .. } if *unit == tv))
        .expect("the escort's answer");
    assert!(
        first_answer > first_arrival,
        "its first shot follows the raider's first arrival: {exchange}"
    );
    assert!(
        asked(&lines[..first_arrival], tv) > 0,
        "the script asked before it was fired on: {:?}",
        lines[..first_arrival]
            .iter()
            .filter(|line| format!("{:?}", line.kind).contains(&format!("ObjId({})", tv.0)))
            .map(|line| format!("{:?}", line.kind))
            .take(12)
            .collect::<Vec<_>>()
    );
    eprintln!("C4_FIELD the exchange: {exchange}");
}

/// **The story** (this module's head). With `script`, the pilot's own mind is written, rated,
/// filed and launched under its owner's licence; without, the rig launches with no mind chosen
/// and flies the house default.
///
/// 1. The escort is launched from its cab. With a filed mind the launch waits behind its claim:
///    the Roll opens the claim in the bay and the launch's seal is the claim's own.
/// 2. The escort stands out on the ground. A second captain's fire order promotes the site; three parties
///    stand in the field; the escort is admitted with its pilot seated, and (filed) the field's
///    seal line names the filed mind, the owner's licence and the claim's card.
/// 3. The pilot marks the raider's party Hostile, sets fire free, and quits. The unit passes to
///    its mind at once.
/// 4. With the raider held Hostile in range and fire free, the mind's unit fires nothing. The
///    filed script asks all the while (it arms and locks); the shot is what is refused.
/// 5. The raider's captain fires on it. From the first arrival the unit answers the raider, and
///    never the third party.
/// 6. It goes home: to its carrier's ramp, out of the field, into the bay by the host's
///    recover. With a filed mind the claim's Close then lands.
async fn story(script: Option<&str>) {
    let (mut world, captain, stranger) = world();
    let (pilot, tv) = (world.pilot, world.tvs[0]);
    let licence = match script {
        Some(source) => Some(file_own_mind(&mut world, "Brawler", source).await),
        None => None,
    };

    // 1. Aboard in the bay, then the launch.
    board(&mut world, tv);
    let claim = match licence {
        Some(licence) => {
            let open = claimed_launch(
                &mut world,
                1,
                &format!("+tv/1 launch mind=filed:{licence} go"),
                licence,
            );
            let seal = tacsim::ic_claim_seal(world.rig.tw.world(), open).expect("the claim's seal");
            Some((open, seal))
        }
        None => {
            let _ = crate::excursions::act_go(&mut world.rig, pilot, "+tv/1 launch go");
            let _ = crate::excursions::finish(&mut world.rig, tv, pilot);
            None
        }
    };
    // 2. The field, over the field harness: the two other parties, the promotion, the seal.
    // The combined world carries no geology key (its ground reads dark, so no rig drives in
    // it), and the escort is stood at its cell by staff, its pilot in the cab, as the M8
    // world's other field stories stand their rigs (`s4b3_*`).
    let (mut rig, kept) = into_field_rig_kept(world);
    crate::driving_rig::to_cell_on(&mut rig, SITE.1, tv, WARD_AT);
    let third = armed(&mut rig, "Red Hound", THIRD_AT, 90_000);
    let raider = armed(&mut rig, "Grey Hound", RAIDER_AT, 0);
    seat(&mut rig, third, captain);
    seat(&mut rig, raider, stranger);
    let before = mush_tactics::module::unit_of(rig.tw.world(), tv)
        .expect("the escort's unit")
        .clone();
    // All three are players at their controls when the field opens. A seated pilot who is not
    // connected then is handed to their unit's mind at the admission (doc 45 cut AP.5), and the
    // story needs the raider's captain to type their own shots.
    for player in [pilot, captain, stranger] {
        let _ = rig.tw.connect(player);
    }
    let mut rx: Receiver<ArenaOut> = egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();
    for _ in 0..3 {
        let _ = crate::field_host_rig::second_composed(&mut rig);
    }
    let (out, queued) = crate::driving_rig::command(&mut rig, captain, "+field/fire E");
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    crate::driving_rig::land_composed(&mut rig, &queued[0]).expect("the promotion lands");
    assert!(
        until_composed(&mut rig, &mut rx, &mut lines, 120, |rig, _| task(rig)
            .is_some())
        .await,
        "the field's task opens"
    );
    let opened = std::time::Instant::now();
    let live = tacsim::field::live_on(rig.tw.world(), &crate::driving_rig::cid(SITE.1))
        .expect("the live record");
    let parties: std::collections::BTreeSet<_> = [tv, raider, third]
        .into_iter()
        .filter_map(|unit| live.party_of(unit))
        .collect();
    assert_eq!(parties.len(), 3, "three parties stand in the field");
    assert!(
        live.units
            .iter()
            .any(|row| row.unit == tv && row.pilot.as_ref().is_some_and(|(p, _)| *p == pilot)),
        "the escort is admitted with its pilot seated"
    );
    // 3. The raider's party marked Hostile and fire free, from the cab.
    let letter = live.party_of(raider).expect("the raider's party").letter();
    let marked =
        crate::driving_rig::run(&mut rig.tw, pilot, &format!("+field/mark {letter}=hostile"));
    let free = crate::driving_rig::run(&mut rig.tw, pilot, "FIRE FREE");
    let hostile_in_view = |rig: &crate::driving_rig::Rig| {
        frame(rig, pilot).is_some_and(|frame| {
            frame.weapons_free
                && frame
                    .contacts
                    .iter()
                    .any(|contact| contact.relation == Relation::Hostile)
        })
    };
    assert!(
        until_composed(&mut rig, &mut rx, &mut lines, 60, |rig, _| hostile_in_view(
            rig
        ))
        .await,
        "the escort is fire free with the raider Hostile in view: {marked}; {free}; {:?}",
        frame(&rig, pilot).map(|frame| (frame.weapons_free, frame.contacts.len()))
    );
    assert_eq!(fired(&lines, tv), 0, "the pilot fired nothing");
    eprintln!(
        "C4_FIELD t+{} s: fire free, the raider Hostile in view",
        opened.elapsed().as_secs()
    );
    // The raider's captain has their own picture of the ground before the pilot leaves, so
    // their fire in step 5 starts when the story says and not when a first frame arrives.
    let seen = until_composed(&mut rig, &mut rx, &mut lines, 90, |rig, _| {
        frame(rig, stranger).is_some()
    })
    .await;
    eprintln!(
        "C4_FIELD t+{} s: the raider's captain has a frame: {seen}",
        opened.elapsed().as_secs()
    );
    assert!(seen, "the raider's captain is sent a cockpit frame");
    // The pilot quits.
    quit(&mut rig, pilot);
    assert!(
        until_composed(&mut rig, &mut rx, &mut lines, 30, |rig, _| {
            minded(rig.tw.world(), tv) && !bound(rig, pilot)
        })
        .await,
        "a clean QUIT hands the unit to its mind at once: {:?}",
        away_of(rig.tw.world(), tv)
    );

    // 4. Nobody has fired on it: for five seconds it stands with a Hostile in range and
    // fire free. The field's black box reaches the host in batches, so what the unit did in
    // this interval is read from the whole log afterwards, by order: step 5 for the filed
    // script, the end of the story for the house mind.
    let _ = until_composed(&mut rig, &mut rx, &mut lines, 5, |_, _| false).await;
    eprintln!(
        "C4_FIELD t+{} s: the quiet interval is over",
        opened.elapsed().as_secs()
    );

    // 5. The raider's captain fires on it for fifteen seconds; from the first arrival it
    // answers the raider (read from the black box at the end). The filed script stands for
    // this; the house mind is already on its way home, and MW.7's own story of an answer
    // (`field_mw7`) is a unit with no carrier to go to.
    if script.is_some() {
        raided(&mut rig, &mut rx, &mut lines, (raider, stranger), 15).await;
    }

    eprintln!("C4_FIELD t+{} s: going home", opened.elapsed().as_secs());
    // 6. It goes home: the ramp, out of the field, the bay.
    let out = until_composed(&mut rig, &mut rx, &mut lines, 420, |rig, _| {
        !in_field(rig, tv)
            && crate::field_host_rig::site_record_at(rig, SITE.1)
                .is_some_and(|record| record.exited(tv))
    })
    .await;
    assert!(
        out,
        "the mind takes the escort out at its carrier's ramp: last at {:?}, away {:?}, in the field {}",
        last_cell(&lines, tv),
        away_of(rig.tw.world(), tv),
        in_field(&rig, tv)
    );
    let home = until_composed(&mut rig, &mut rx, &mut lines, 300, |rig, _| {
        let due = {
            let h = crate::driving_rig::handles(&rig.tw);
            crate::driving_rig::typed(
                &mut rig.tw,
                &h.families.excursions,
                &tacsim::surface::excursion::RigKey(tv),
            )
            .and_then(|record| record.act.map(|act| act.due_ic))
        };
        if let Some(due) = due {
            crate::driving_rig::to_ic(rig, due);
        }
        matches!(
            crate::driving_rig::parent(&rig.tw, tv),
            mush_sim::SpatialParent::Interior { .. }
        )
    })
    .await;
    eprintln!(
        "C4_FIELD t+{} s: out of the field",
        opened.elapsed().as_secs()
    );
    assert!(
        home,
        "the host's recover puts the escort in its carrier's bay"
    );
    if let Some((open, _)) = claim {
        assert!(
            until_composed(&mut rig, &mut rx, &mut lines, 600, |rig, _| {
                tacsim::ic_claim_closed(rig.tw.world(), open).is_some()
            })
            .await,
            "the claim's Close lands once the unit is home"
        );
    }
    // The field admitted it under its claim: the seal line on the field's log, read here
    // and not at the admission, because a field nobody fires in closes after two quiet
    // minutes and the line reaches the host in its own time.
    if let Some((_, seal)) = &claim {
        let sealed = |lines: &[LedgerEvent]| {
            lines.iter().find_map(|line| match &line.kind {
                EventKind::MindSealed {
                    unit,
                    mind,
                    licence,
                    licence_state,
                    card_digest,
                    ..
                } if *unit == tv => Some((
                    mind.clone(),
                    licence.clone(),
                    *licence_state,
                    card_digest.clone(),
                )),
                _ => None,
            })
        };
        assert!(
            until_composed(&mut rig, &mut rx, &mut lines, 300, |_, lines| sealed(lines)
                .is_some())
            .await,
            "the escort's seal line lands on the field's log"
        );
        let (mind, named, state, card) = sealed(&lines).expect("the seal line");
        let facts = tacsim::surface::black_box::SealFacts::of(
            seal,
            tacsim::minds::licence_state::SealLicenceState::Active,
        );
        assert_eq!(mind, facts.mind, "the field admits the claim's own mind");
        assert_eq!(
            named.as_str(),
            licence.expect("filed").to_string(),
            "under the pilot's owner's licence"
        );
        assert_eq!(
            state,
            Some(tacsim::minds::licence_state::SealLicenceState::Active)
        );
        assert_eq!(card, facts.card_digest, "the claim's card, not the ROM's");
    }

    // What the unit did, from its own record: the recover wrote the hull the field left, so a
    // round spent or a plate struck shows as a cell that differs from the hull that went out.
    let after = mush_tactics::module::unit_of(rig.tw.world(), tv)
        .expect("the escort's unit")
        .clone();
    let changed: Vec<String> = before
        .cells()
        .filter(|(addr, cell)| after.cell(addr).map(|now| now.current) != Some(cell.current))
        .map(|(addr, cell)| {
            format!(
                "{addr:?}: {} -> {:?}",
                cell.current,
                after.cell(addr).map(|now| now.current)
            )
        })
        .collect();
    eprintln!("C4_FIELD the escort's hull, out and home: {changed:?}");
    if script.is_some() {
        // The whole exchange, read once the black box has caught up. A quiet field's box
        // reaches the host as its chunks fill, and whole when the field closes; with nobody
        // firing, the field closes after its two quiet minutes. So the story waits for the
        // close (or for a position summary that no longer lists the unit, which was written
        // after it left), then for the last of the box.
        let caught_up = until_composed(&mut rig, &mut rx, &mut lines, 600, |rig, lines| {
            task(rig).is_none()
                || lines
                    .iter()
                    .rev()
                    .find_map(|line| match &line.kind {
                        EventKind::PositionSummary { units } => {
                            Some(!units.iter().any(|row| row.0 == tv))
                        }
                        _ => None,
                    })
                    .unwrap_or(false)
        })
        .await;
        assert!(caught_up, "the field closes and its log is whole");
        let _ = until_composed(&mut rig, &mut rx, &mut lines, 5, |_, _| false).await;
        eprintln!(
            "C4_FIELD t+{} s: the black box is in hand, {} lines",
            opened.elapsed().as_secs(),
            lines.len()
        );
        assert_eq!(
            fired_at(&lines, tv, third),
            0,
            "it never fired on the party that never fired on it"
        );
        answered_in_order(&lines, (tv, raider, third));
        assert!(
            !changed.is_empty(),
            "the escort that fought came home with rounds spent or plates struck"
        );
    } else {
        // The house mind, with a Hostile in range and fire free, spent nothing and was not
        // struck: the hull that came home is the hull that went out. (The dispatch story reads
        // the black box; the box of a field this quiet takes minutes to reach the host.)
        assert!(
            changed.is_empty(),
            "the house mind fired on nobody: {changed:?}"
        );
    }
    drop(kept);
    crate::driving_rig::no_refused_writes(&rig.tw);
}

/// **MW.7's rule on the launch composition, for the house mind** (this module's head): no mind
/// chosen, no rating; the default sweep runs it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn c4_the_house_mind_takes_an_absent_pilots_escort_home_on_the_launch() {
    story(None).await;
}

/// **The owed story** (doc 44 §7.1, MW.7's second story, as a field story through the
/// Registry's claim path): a player's own filed mind, written to fire first, is refused the
/// shot, answers the unit that hits it, and is taken home, in a field of three parties.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "MW.5 story: a genuine Registry rating; the CI dispatch runs it"]
async fn c4_a_filed_mind_flies_an_absent_pilots_escort_and_is_taken_home() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("warn,ic_claim=debug")
        .with_test_writer()
        .try_init();
    story(Some(&fires_first())).await;
}

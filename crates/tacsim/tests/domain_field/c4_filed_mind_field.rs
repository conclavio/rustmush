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

/// **The pilot's own script, written to fire first.** At every think it picks a contact it has
/// told apart that is not a friend (one that reads hostile, where there is one), arms the
/// cannon on a lock, locks it and fires. It asks whether or not anyone has fired on it. Told to
/// go home, it stands and keeps asking for forty seconds, then drives for its party's entry,
/// which is its carrier's ramp.
fn fires_first() -> String {
    format!(
        "fn think(v, c) {{ \
           let id = (); \
           for k in v.contacts {{ if k.identified && !k.friendly {{ id = k.id; }} }} \
           for k in v.contacts {{ if k.relation == \"hostile\" {{ id = k.id; }} }} \
           if id != () {{ arm(\"cannon\", \"lock\", 100, 15, 0, 0, 4294967295); lock(id); fire(\"cannon\", 1); }} \
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
    loop {
        world.pulse();
        let done = world.rows("rating_receipts").into_iter().any(|wire| {
            serde_json::from_value::<mush_roll::commerce::rating::RatingReceipt>(wire)
                .is_ok_and(|receipt| receipt.subject().revision() == identity)
        });
        if done {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the Registry did not rate {name}"
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    eprintln!("C4_FIELD rated {name} in {} s", started.elapsed().as_secs());
    // The filing: the charter's shell first, then the filing itself, each the pilot's own
    // submission landed on the composed path. The Registry releases its job a pulse after the
    // receipt, so the first line is asked again until it is taken.
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut filed = 0;
    while filed < 2 {
        let (said, queued) =
            crate::excursions::command(&mut world.rig, pilot, &format!("@mind/file {name}@1"));
        eprintln!("C4_FIELD file -> {said}");
        if queued.is_empty() {
            assert!(Instant::now() < deadline, "the filing never staged: {said}");
            world.pulse();
            tokio::time::sleep(Duration::from_millis(20)).await;
            continue;
        }
        for submit in &queued {
            crate::excursions::land(&mut world.rig, submit)
                .unwrap_or_else(|refused| panic!("@mind/file: {refused}"));
        }
        filed += 1;
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

/// **The raider's captain fires on the escort until it answers** (the story's step 5): a lock
/// kept on the escort and a shot a wall second, then the order of things on the black box: the
/// escort's first shot follows the raider's first arrival on it.
async fn raided(
    rig: &mut crate::driving_rig::Rig,
    rx: &mut Receiver<ArenaOut>,
    lines: &mut Vec<LedgerEvent>,
    (tv, raider, third): (ObjId, ObjId, ObjId),
    stranger: ObjId,
) {
    let gun = weapon_of(rig, raider);
    let mut answered = false;
    for _ in 0..90 {
        // The raider's captain keeps a lock on the escort: the contact on their own frame that
        // is not the third captain's hull, which never moves.
        let escort = frame(rig, stranger).and_then(|frame| {
            let lock = frame.lock;
            frame
                .contacts
                .iter()
                .find(|contact| {
                    contact.cell.is_some_and(|(x, y)| {
                        i64::from(x).abs_diff(THIRD_AT.0) > 2
                            || i64::from(y).abs_diff(THIRD_AT.1) > 2
                    })
                })
                .map(|contact| (contact.id, contact.designator.clone(), lock))
        });
        if let Some((id, designator, lock)) = escort
            && lock != Some(id)
        {
            let _ = crate::driving_rig::run(&mut rig.tw, stranger, &format!("LOCK {designator}"));
        }
        let _ = crate::driving_rig::run(&mut rig.tw, stranger, &format!("FIRE {gun}"));
        if until_composed(rig, rx, lines, 1, |_, lines| {
            fired_at(lines, tv, raider) > 0
        })
        .await
        {
            answered = true;
            break;
        }
    }
    let _ = crate::driving_rig::run(&mut rig.tw, stranger, &format!("FIRE {gun} OFF"));
    let exchange = format!(
        "escort fired {} (at the raider {}, at the third {}); raider fired {}, arrived {}; away {:?}; in the field {}; the raider sees {:?}",
        fired(lines, tv),
        fired_at(lines, tv, raider),
        fired_at(lines, tv, third),
        fired(lines, raider),
        arrived(lines, raider, tv),
        away_of(rig.tw.world(), tv),
        in_field(rig, tv),
        frame(rig, stranger).map(|frame| (
            frame.lock,
            frame
                .contacts
                .iter()
                .map(|contact| (contact.id, contact.cell, contact.relation))
                .collect::<Vec<_>>()
        )),
    );
    assert!(answered, "it answers the unit that fired on it: {exchange}");
    let first_arrival = lines
        .iter()
        .position(|line| {
            matches!(&line.kind, EventKind::Impact { unit, target, .. }
                if *unit == raider && *target == tv)
        })
        .expect("the raider's fire arrived");
    let first_answer = lines
        .iter()
        .position(|line| matches!(&line.kind, EventKind::Fired { unit, .. } if *unit == tv))
        .expect("the escort's answer");
    assert!(
        first_answer > first_arrival,
        "its first shot follows the raider's first arrival: {exchange}"
    );
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
    let _ = rig.tw.connect(pilot);
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

    // 3. The raider's party marked Hostile, fire free, and the pilot quits.
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
    quit(&mut rig, pilot);
    assert!(
        until_composed(&mut rig, &mut rx, &mut lines, 30, |rig, _| {
            minded(rig.tw.world(), tv) && !bound(rig, pilot)
        })
        .await,
        "a clean QUIT hands the unit to its mind at once: {:?}",
        away_of(rig.tw.world(), tv)
    );
    let handed = lines.len();

    // 4. Nobody has fired on it: it fires nothing, though (filed) the script asks.
    let _ = until_composed(&mut rig, &mut rx, &mut lines, 15, |_, _| false).await;
    assert_eq!(
        fired(&lines, tv),
        0,
        "with a Hostile in range and fire free, the mind's unit fires on nobody"
    );
    if script.is_some() {
        assert!(
            asked(&lines[handed..], tv) > 0,
            "the script asks all the while: {:?}",
            lines[handed..]
                .iter()
                .filter(|line| format!("{:?}", line.kind).contains(&format!("ObjId({})", tv.0)))
                .map(|line| format!("{:?}", line.kind))
                .take(12)
                .collect::<Vec<_>>()
        );
    }

    // 5. The raider's captain fires on it; from the first arrival it answers the raider. The
    // filed script stands for this; the house mind is already on its way home, and MW.7's own
    // story of an answer (`field_mw7`) is a unit with no carrier to go to.
    if script.is_some() {
        raided(&mut rig, &mut rx, &mut lines, (tv, raider, third), stranger).await;
    }

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
    assert_eq!(
        fired_at(&lines, tv, third),
        0,
        "it never fired on the party that never fired on it"
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

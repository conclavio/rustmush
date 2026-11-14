//! **Doc 44 unit MW.11's acceptance, on the bubble's own step** ([`super::step_bubble`], the
//! function the space tick calls for each bubble): the same encounter, taken from before a
//! decision and run on hosts whose workers answer at different times, gives the same
//! commands, the same mind memory and the same outcome; and a step whose thinks have not
//! answered is held without the stepping thread waiting.

#![cfg(test)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use mush_testkit::TestWorld;

use super::*;
use crate::arena::mind::MindRunner;
use crate::minds::MindPool;
use crate::space::bubble::BubbleInputEntry;
use crate::space::picket::{
    CUSTOMS_CORVETTE_PARTY, KirkwoodPicketState, PicketInspectionOutcome, PicketInspectionStatus,
};
use crate::space::thinks::test_support::{Gate, Gated};

const CARRIER: ObjId = ObjId(10);
const ESCORT: ObjId = ObjId(20);
const RUNNER: ObjId = ObjId(21);

fn carrier_entry(hull: ObjId, party: PartyId) -> BubbleCarrierEntry {
    BubbleCarrierEntry {
        hull,
        ship_id: format!("hull.{}", hull.0),
        party,
        leg_cursor: 1,
        departed_at_ic_s: 1_000,
        last_synced_propellant_kg: 0,
        approaching_derelict: false,
        holding_for_salvage: false,
        holding_for_salvage_until_wall_s: None,
    }
}

/// A bubble record over `bubble`, as a world opens one.
fn active_over(
    bubble: LocalSpaceBubble,
    domain: DomainId,
    carriers: BTreeMap<ObjId, BubbleCarrierEntry>,
    picket: Option<KirkwoodPicketState>,
    interdictor: (ObjId, String),
) -> ActiveInterdictionBubble {
    let first = carriers.keys().next().copied().unwrap_or(ObjId(0));
    let tape = BubbleInputTape::new(&bubble);
    let interdictor_seal = if picket.is_some() {
        None
    } else {
        super::super::picket::pack_cast_seal(
            crate::minds::content::cast::CastActor::Interdictor,
            interdictor.0,
        )
    };
    ActiveInterdictionBubble {
        hull: first,
        interdictor_hull: interdictor.0,
        domain,
        carrier_ship_id: format!("hull.{}", first.0),
        interdictor_ship_id: interdictor.1,
        leg_cursor: 1,
        opened_ic_s: 1_000,
        steps: 0,
        cleared: false,
        snapshots: vec![bubble.snapshot()],
        bubble,
        tape,
        picket,
        approaching_derelict: false,
        holding_for_salvage: false,
        last_stepped_wall_s: 0,
        last_synced_propellant_kg: 0,
        carriers,
        ship_hulls: BTreeMap::new(),
        interdictor_memory: Vec::new(),
        interdictor_think_seq: 0,
        interdictor_seal,
        interdictor_contacts: super::super::view::ContactBook::default(),
        stood_down: std::collections::BTreeSet::new(),
        thinks_owed: None,
    }
}

/// **A customs picket with three minds**: the corvette and the raider the cast names, and
/// one Dray closing on the corvette whose crew has no link, so the backup mind flies her.
fn picket_encounter() -> (
    ActiveInterdictionBubble,
    BTreeMap<ObjId, SpaceLinkdeadState>,
) {
    let mut bubble = LocalSpaceBubble::new(
        "bubble.mw11.kirkwood",
        "domain.7",
        "Kirkwood Customs Picket Bubble",
        1_000_000,
    );
    bubble.insert_ship(SpaceShipState::new_dray(
        "hull.10",
        "Salt Moth",
        CARRIER_PARTY,
        [0, 0, 0],
        [25_000, 0, 0],
    ));
    let picket =
        super::super::picket::build_kirkwood_picket_ships(&mut bubble, DomainId(7), 1, "hull.10")
            .expect("the picket's ships build");
    let interdictor = (picket.raider_hull, picket.raider_ship_id.clone());
    let mut carriers = BTreeMap::new();
    carriers.insert(CARRIER, carrier_entry(CARRIER, CARRIER_PARTY));
    let active = active_over(bubble, DomainId(7), carriers, Some(picket), interdictor);
    let mut linkdead = BTreeMap::new();
    linkdead.insert(
        CARRIER,
        SpaceLinkdeadState {
            hull: CARRIER,
            grace_started_wall_s: 1,
            grace_started_ic_s: 1,
            pdc_held: true,
            grace_expired: true,
            queued_return_port: None,
            mind_memory: Vec::new(),
            mind_last_trace: None,
            mind_contacts: super::super::view::ContactBook::default(),
            mind_think_seq: 0,
            backup_act: super::super::backup::BackupAct::Standby,
            return_retry_at_wall_s: 0,
            next_think_at_wall_s: 0,
        },
    );
    (active, linkdead)
}

/// **An interdiction with no customs**: the interdictor, dark and at rest, an escort that
/// is nearly hove to and a runner that is not, each hostile to it both ways.
fn interdiction_encounter() -> (
    ActiveInterdictionBubble,
    BTreeMap<ObjId, SpaceLinkdeadState>,
) {
    const INTERDICTOR: &str = "interdictor.5.1";
    let mut bubble = LocalSpaceBubble::new("bubble.mw11.int", "domain.5", "Test", 1_000);
    let mut interdictor = SpaceShipState::new_corvette(
        INTERDICTOR,
        "Interdictor Picket",
        INTERDICTOR_PARTY,
        [0, 0, 0],
        [0, 0, 0],
    );
    super::super::custody::apply_space_unit_overlay(
        &mut interdictor,
        &super::super::custody::space_unit_overlay_for_id("tacsim.unit.corvette_interdictor"),
    );
    interdictor.emcon = EmconPosture::Ghost;
    bubble.insert_ship(interdictor);
    let runner_party = admitted_party(3);
    let mut carriers = BTreeMap::new();
    for (hull, party, pos, vel) in [
        (ESCORT, CARRIER_PARTY, [-12_000, 2_000, 0], [3_000, 0, 0]),
        (RUNNER, runner_party, [-14_000, -2_000, 500], [25_000, 0, 0]),
    ] {
        let entry = carrier_entry(hull, party);
        bubble.insert_ship(SpaceShipState::new_dray(
            &entry.ship_id,
            "Carrier",
            party,
            pos,
            vel,
        ));
        bubble.set_party_relation(party, INTERDICTOR_PARTY, Some(Mark::Hostile));
        bubble.set_party_relation(INTERDICTOR_PARTY, party, Some(Mark::Hostile));
        carriers.insert(hull, entry);
    }
    bubble.set_party_relation(CARRIER_PARTY, runner_party, Some(Mark::Friendly));
    bubble.set_party_relation(runner_party, CARRIER_PARTY, Some(Mark::Friendly));
    let active = active_over(
        bubble,
        DomainId(5),
        carriers,
        None,
        (ObjId(900), INTERDICTOR.to_owned()),
    );
    (active, BTreeMap::new())
}

/// **What the world does to the encounter from outside, keyed on the encounter's own
/// state**, so it falls at the same point of every run: customs rule on a carrier that has
/// hove to, fining and clearing her, as `+helm/submit` and the ruling do in a world.
fn world_acts(active: &mut ActiveInterdictionBubble) {
    let Some(picket) = active.picket.as_mut() else {
        return;
    };
    if picket.inspection_status_for(CARRIER) == PicketInspectionStatus::HoveToReadyForInspection
        && picket.inspection_outcome_for(CARRIER).is_none()
    {
        picket.set_inspection_status_for(CARRIER, PicketInspectionStatus::ClearedAfterFine);
        picket.set_inspection_outcome_for(
            CARRIER,
            PicketInspectionOutcome::Fined {
                contract_id: 42,
                fine_minor: 200_000,
                unlicensed_strategic_ore_kg: 4_000,
                front_id: Some(1),
            },
        );
        active.bubble.set_party_relation(
            CUSTOMS_CORVETTE_PARTY,
            CARRIER_PARTY,
            Some(Mark::Friendly),
        );
    }
}

/// One run of an encounter: where it ended, and every command its tape took.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Run {
    active: ActiveInterdictionBubble,
    linkdead: BTreeMap<ObjId, SpaceLinkdeadState>,
    log: Vec<BubbleInputEntry>,
}

/// A copy of `run`, kept off the caller's frame.
fn kept(run: &Run) -> Box<Run> {
    Box::new(run.clone())
}

/// What a run does while a step is held.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WhileHeld {
    /// The host is a rig's and waits: no step is ever held.
    NeverHeld,
    /// The tick is fired again at the same wall second, as a rig that will not move its
    /// clock past a held step does.
    ClockStands,
    /// The wall clock runs on, a second a tick, as the server's does.
    ClockRuns,
}

/// The runner each digest of `digests` builds on `host`, put behind `gate`.
fn gate_every_runner(host: &ThinkHost, digests: &[[u8; 32]], gate: &Arc<Gate>) {
    let view = Arc::new(crate::space::sortie::rig::view(ObjId(1)));
    for digest in digests {
        let _ = host.job(None, ObjId(1), *digest, 0, &view, &[]);
        let inner = host.kept_runner(digest).expect("the doctrine has a runner");
        let gated: Arc<dyn MindRunner> = Arc::new(Gated {
            inner,
            gate: Arc::clone(gate),
        });
        host.install_runner(*digest, gated);
    }
}

/// Every digest an encounter's minds think with.
fn digests_of(active: &ActiveInterdictionBubble) -> Vec<[u8; 32]> {
    let mut digests = vec![crate::minds::content::linkdead_return_digest().0];
    if let Some(picket) = &active.picket {
        digests.extend(picket.corvette_seal.iter().map(|seal| seal.source_sha256));
        digests.extend(picket.raider_seal.iter().map(|seal| seal.source_sha256));
    }
    digests.extend(
        active
            .interdictor_seal
            .iter()
            .map(|seal| seal.source_sha256),
    );
    digests
}

/// **Drive `start` for `steps` more steps on `host`**, a tick a wall second from `wall`.
/// With `gate`, each step's thinks are parked until the tick has found the step held
/// `held_ticks(step)` times; what the clock does meanwhile is `while_held`.
#[allow(clippy::too_many_arguments)]
fn drive(
    start: &Run,
    host: &ThinkHost,
    w: &World,
    steps: u32,
    mut wall: i64,
    while_held: WhileHeld,
    gate: Option<&Arc<Gate>>,
    held_ticks: &dyn Fn(u32) -> u32,
) -> Run {
    let mut run = start.clone();
    let paused = std::collections::BTreeSet::new();
    let orders = mush_sim::tick::StandingOrderBook::default();
    let until = Instant::now() + Duration::from_secs(120);
    for _ in 0..steps {
        wall += 1;
        world_acts(&mut run.active);
        let step = run.active.steps;
        let mut held = 0_u32;
        loop {
            assert!(Instant::now() < until, "the encounter did not finish");
            let tick = BubbleTick {
                frozen: false,
                wall_now_s: wall,
                ic_now_s: 1_000 + wall * 24,
                paused_hulls: &paused,
                standing_orders: &orders,
                host,
                store: None,
                w,
            };
            let before = (
                run.active.bubble.state_hash(),
                run.active.tape.entries.len(),
            );
            let mut out = BubbleTickOut::default();
            // The gate is shut before the step that will be taken hands its next thinks
            // over, so they park too.
            if step_bubble(&mut run.active, &tick, &mut run.linkdead, &mut out) {
                break;
            }
            assert_ne!(while_held, WhileHeld::NeverHeld, "a rig's host held a step");
            assert_eq!(
                before,
                (
                    run.active.bubble.state_hash(),
                    run.active.tape.entries.len()
                ),
                "a held step leaves the bubble and its tape as they were"
            );
            held += 1;
            if while_held == WhileHeld::ClockRuns {
                wall += 1;
            }
            if held >= held_ticks(step)
                && let (Some(gate), Some(owed)) = (gate, run.active.thinks_owed)
            {
                // Let the workers through, wait until the batch has answered, and shut the
                // gate again for the thinks the step is about to collect.
                gate.open();
                let (owner, mark) = run.active.think_owner(owed);
                while !host.is_answered(owner, mark) {
                    assert!(Instant::now() < until, "the batch never answered");
                    std::thread::yield_now();
                }
                gate.close();
            }
        }
        assert_eq!(run.active.steps, step + 1);
        // Keep every command, and restart the tape, so its own compaction never drops one.
        run.log.append(&mut run.active.tape.entries);
        run.active.tape = BubbleInputTape::new(&run.active.bubble);
    }
    run
}

/// `run` with what the wall clock alone decides taken out: the IC second each step was
/// stamped with, the wall second of the last step, and the snapshots that fold them.
fn without_the_clock(mut run: Run) -> Run {
    for entry in &mut run.log {
        if let BubbleInputCommand::StepWall { ic_now_s, .. } = &mut entry.command {
            *ic_now_s = 0;
        }
    }
    run.active.bubble.ic_timestamp_s = 0;
    run.active.last_stepped_wall_s = 0;
    run.active.snapshots.clear();
    run.active.tape = BubbleInputTape::new(&run.active.bubble);
    run
}

/// How many commands of `log` a mind issued: everything but the steps.
fn mind_commands(log: &[BubbleInputEntry]) -> usize {
    log.iter()
        .filter(|entry| !matches!(entry.command, BubbleInputCommand::StepWall { .. }))
        .count()
}

/// **The acceptance, for one encounter.** It is flown `lead_in` steps on a rig's host, to a
/// point where its minds have thinks out; that point is "before a decision". From there it
/// is run `steps` more on each host and every run is held to the first.
fn same_whatever_the_workers_timing(
    name: &str,
    start: (
        ActiveInterdictionBubble,
        BTreeMap<ObjId, SpaceLinkdeadState>,
    ),
    lead_in: u32,
    steps: u32,
) -> Run {
    let tw = TestWorld::new();
    let w = tw.world();
    let digests = digests_of(&start.0);
    let fresh = Run {
        active: start.0,
        linkdead: start.1,
        log: Vec::new(),
    };
    let never = |_: u32| 0_u32;
    let before = drive(
        &fresh,
        &ThinkHost::on_pool(MindPool::new(2)),
        w,
        lead_in,
        100,
        WhileHeld::NeverHeld,
        None,
        &never,
    );
    assert!(
        before.active.thinks_owed.is_some(),
        "{name}: the minds have thinks out at the point the runs start from"
    );
    let wall = 100 + i64::from(lead_in);

    // The reference: every think run on the stepping thread, at the moment it is collected.
    let reference = drive(
        &before,
        &ThinkHost::inline(),
        w,
        steps,
        wall,
        WhileHeld::NeverHeld,
        None,
        &never,
    );
    assert!(
        mind_commands(&reference.log) >= 4,
        "{name}: the minds decided something: {} commands",
        mind_commands(&reference.log)
    );

    // A rig's host, which waits for its workers: one worker, and four.
    for workers in [1_u32, 4] {
        let host = ThinkHost::on_pool(MindPool::new(workers));
        let run = drive(
            &before,
            &host,
            w,
            steps,
            wall,
            WhileHeld::NeverHeld,
            None,
            &never,
        );
        assert_eq!(run, reference, "{name}: on {workers} waiting worker(s)");
        assert_eq!(host.stats().held, 0);
    }

    // The server's host, which holds: every step's thinks are parked while the tick finds
    // the step held nought to three times, and the tick is fired again at the same second.
    for workers in [1_u32, 3] {
        let host = ThinkHost::on_pool(MindPool::new(workers));
        host.hold_late_steps();
        let gate = Arc::new(Gate::default());
        gate_every_runner(&host, &digests, &gate);
        let pattern = |step: u32| (step.wrapping_mul(7).wrapping_add(workers)) % 4;
        let run = drive(
            &before,
            &host,
            w,
            steps,
            wall,
            WhileHeld::ClockStands,
            Some(&gate),
            &pattern,
        );
        assert_eq!(
            run, reference,
            "{name}: held steps on {workers} worker(s), the clock standing"
        );
        assert!(
            host.stats().held >= u64::from(steps),
            "{name}: steps were held: {}",
            host.stats().held
        );
    }

    // The same, with the wall clock running on under the held steps as the server's does.
    // Each held step is taken as it first fell due; every later step is stamped with a
    // later IC second than the reference's, and nothing else differs.
    let host = ThinkHost::on_pool(MindPool::new(2));
    host.hold_late_steps();
    let gate = Arc::new(Gate::default());
    gate_every_runner(&host, &digests, &gate);
    let pattern = |step: u32| 1 + step % 3;
    let late = drive(
        &before,
        &host,
        w,
        steps,
        wall,
        WhileHeld::ClockRuns,
        Some(&gate),
        &pattern,
    );
    assert!(late.active.last_stepped_wall_s > reference.active.last_stepped_wall_s);
    assert_eq!(
        without_the_clock(late),
        without_the_clock(reference.clone()),
        "{name}: held steps with the clock running"
    );
    reference
}

/// **Doc 44 unit MW.11 (Q-44-28): the same encounter, run again from before a decision
/// with the workers' timing varied, gives the same commands, the same mind memory and the
/// same outcome.** A customs picket: the corvette hails, the backup mind of a hull with no
/// crew heaves her to, customs clear her, the raider lights up and opens on her.
#[test]
fn mw11_a_picket_is_the_same_whatever_the_workers_timing() {
    let reference = same_whatever_the_workers_timing("picket", picket_encounter(), 3, 28);
    let picket = reference.active.picket.as_ref().expect("a picket");
    assert!(picket.raider_ambush_launched, "the raider opened");
    let backup = reference.linkdead.get(&CARRIER).expect("her record");
    eprintln!(
        "picket: corvette {} thinks, raider {} thinks, backup {} thinks ({:?}), {} mind commands",
        picket.corvette_think_seq,
        picket.raider_think_seq,
        backup.mind_think_seq,
        backup.backup_act,
        mind_commands(&reference.log),
    );
    assert!(picket.corvette_think_seq >= 5 && picket.raider_think_seq >= 5);
    assert!(!picket.corvette_memory.is_empty(), "the corvette remembers");
    assert!(backup.mind_think_seq >= 5, "the backup mind thought");
    // Doc 47 unit CO.2 (R-47-3): she hove to, and once the raider opened on her she
    // answered it, on `RETURN`, the word every hull holds. Until CO.2 the last act was
    // heaving to.
    assert_eq!(
        backup.backup_act,
        super::super::backup::BackupAct::ReturningFire,
        "she hove to and answered the raider"
    );
    assert!(
        reference
            .active
            .bubble
            .fired_on_by("hull.10")
            .is_some_and(|by| by.contains(&picket.raider_ship_id)),
        "after the raider's fire had arrived on her"
    );
}

/// **Doc 44 unit MW.11 (Q-44-28): the same, for an interdiction**: the interdictor hails,
/// holds for the hull that is slowing and closes on the one that runs.
#[test]
fn mw11_an_interdiction_is_the_same_whatever_the_workers_timing() {
    let reference =
        same_whatever_the_workers_timing("interdiction", interdiction_encounter(), 2, 24);
    eprintln!(
        "interdiction: interdictor {} thinks, {} mind commands",
        reference.active.interdictor_think_seq,
        mind_commands(&reference.log),
    );
    assert!(reference.active.interdictor_think_seq >= 5);
    assert!(
        reference.active.stood_down.contains(&ESCORT),
        "the escort that hove to was stood down"
    );
    assert!(!reference.active.stood_down.contains(&RUNNER));
}

/// **Doc 44 unit MW.11: no think blocks the stepping thread.** On the server's host, with
/// every worker parked, a bubble whose step is due is asked two hundred times and each
/// call returns without the step being taken or anything written; the thread is never
/// waiting on a worker, so the calls are done in a fraction of the time one parked think
/// would have cost it. When the workers are let through the step is taken, as it first
/// fell due.
#[test]
fn mw11_a_step_whose_thinks_are_out_is_held_and_the_thread_goes_on() {
    let tw = TestWorld::new();
    let w = tw.world();
    for (name, (active, linkdead)) in [
        ("picket", picket_encounter()),
        ("interdiction", interdiction_encounter()),
    ] {
        let host = ThinkHost::on_pool(MindPool::new(2));
        host.hold_late_steps();
        let gate = Arc::new(Gate::default());
        gate_every_runner(&host, &digests_of(&active), &gate);
        let mut run = Run {
            active,
            linkdead,
            log: Vec::new(),
        };
        let paused = std::collections::BTreeSet::new();
        let orders = mush_sim::tick::StandingOrderBook::default();
        let tick_at = |wall: i64| BubbleTick {
            frozen: false,
            wall_now_s: wall,
            ic_now_s: 1_000 + wall * 24,
            paused_hulls: &paused,
            standing_orders: &orders,
            host: &host,
            store: None,
            w,
        };
        // The first step owes nothing and is taken; its minds' thinks park at the gate.
        let mut out = BubbleTickOut::default();
        assert!(step_bubble(
            &mut run.active,
            &tick_at(101),
            &mut run.linkdead,
            &mut out
        ));
        let owed = run.active.thinks_owed.expect("the minds have thinks out");
        // A worker is known to be parked on a think before the held ticks are counted. (The
        // ticks below are held whether or not a worker has got that far; this makes the
        // test's claim, that the thread goes on beside a parked worker, true on a runner
        // whose few cores are slow to wake one.)
        let woken = Instant::now() + Duration::from_secs(60);
        while gate.arrived() < 1 {
            assert!(Instant::now() < woken, "{name}: no worker reached the gate");
            std::thread::yield_now();
        }
        // On the heap: the lint job holds a test's frame under 64 KiB, and this one holds
        // both encounters and the run already.
        let before = kept(&run);
        let started = Instant::now();
        for n in 0..200_i64 {
            let mut out = BubbleTickOut::default();
            assert!(
                !step_bubble(
                    &mut run.active,
                    &tick_at(102 + n),
                    &mut run.linkdead,
                    &mut out
                ),
                "{name}: the step is held while its thinks are out"
            );
        }
        let spent = started.elapsed();
        assert_eq!(run, *before, "{name}: a held step writes nothing");
        assert_eq!(host.stats().held, 200);
        assert!(
            spent < Duration::from_secs(10),
            "{name}: two hundred held ticks took {spent:?}"
        );
        // The step that fell due at second 102 is the one taken, whenever it is taken.
        let (owner, mark) = run.active.think_owner(owed);
        assert_eq!(
            host.held_step(owner, mark),
            Some(Due {
                dt_wall_ms: 1_000,
                ic_now_s: 1_000 + 102 * 24,
            })
        );
        gate.open();
        let until = Instant::now() + Duration::from_secs(60);
        while !host.is_answered(owner, mark) {
            assert!(Instant::now() < until, "{name}: the batch never answered");
            std::thread::yield_now();
        }
        let mut out = BubbleTickOut::default();
        assert!(step_bubble(
            &mut run.active,
            &tick_at(400),
            &mut run.linkdead,
            &mut out
        ));
        let step = run
            .active
            .tape
            .entries
            .iter()
            .rev()
            .find_map(|entry| match entry.command {
                BubbleInputCommand::StepWall {
                    dt_wall_ms,
                    ic_now_s,
                } => Some((dt_wall_ms, ic_now_s)),
                _ => None,
            });
        assert_eq!(
            step,
            Some((1_000, 1_000 + 102 * 24)),
            "{name}: the held step is taken as it fell due, not as the 298 seconds since"
        );
        assert_eq!(run.active.last_stepped_wall_s, 400);
    }
}

/// **Doc 44 unit MW.11: a bubble read back with thinks owed collects them again.** A
/// restart loses the batch in flight and keeps the bubble's record; the bubble then thinks
/// on the state it kept and goes on exactly as one that was never restarted.
#[test]
fn mw11_a_bubble_read_back_with_thinks_owed_collects_them_again() {
    let tw = TestWorld::new();
    let w = tw.world();
    let (active, linkdead) = picket_encounter();
    let fresh = Run {
        active,
        linkdead,
        log: Vec::new(),
    };
    let never = |_: u32| 0_u32;
    let host = ThinkHost::on_pool(MindPool::new(2));
    let unbroken = drive(
        &fresh,
        &host,
        w,
        12,
        100,
        WhileHeld::NeverHeld,
        None,
        &never,
    );

    let host = ThinkHost::on_pool(MindPool::new(2));
    let first = drive(&fresh, &host, w, 5, 100, WhileHeld::NeverHeld, None, &never);
    assert!(first.active.thinks_owed.is_some());
    // The record goes through its stored form, as a restart reads it, and meets a host
    // that has never seen it.
    let stored = serde_json::to_value(&first.active).expect("the record is stored");
    let read_back: ActiveInterdictionBubble =
        serde_json::from_value(stored).expect("the record is read back");
    assert_eq!(read_back.thinks_owed, first.active.thinks_owed);
    let restarted = Run {
        active: read_back,
        linkdead: first.linkdead.clone(),
        log: first.log.clone(),
    };
    let host = ThinkHost::on_pool(MindPool::new(2));
    let resumed = drive(
        &restarted,
        &host,
        w,
        7,
        105,
        WhileHeld::NeverHeld,
        None,
        &never,
    );
    assert_eq!(resumed.log, unbroken.log, "the same commands");
    assert_eq!(
        resumed.active.bubble, unbroken.active.bubble,
        "the same outcome"
    );
    assert_eq!(
        resumed.active.picket, unbroken.active.picket,
        "the same memory"
    );
    assert_eq!(resumed.linkdead, unbroken.linkdead);
}

/// **Doc 44 Q-44-37, unit MW.11: a crew that takes the helm back discards the think the
/// backup mind had out for her**, and the other minds' answers of the same batch still
/// land.
#[test]
fn mw11_a_think_struck_from_a_batch_does_not_land_and_the_rest_do() {
    let tw = TestWorld::new();
    let w = tw.world();
    let (active, linkdead) = picket_encounter();
    let fresh = Run {
        active,
        linkdead,
        log: Vec::new(),
    };
    let never = |_: u32| 0_u32;
    let host = ThinkHost::on_pool(MindPool::new(2));
    let mut run = drive(&fresh, &host, w, 4, 100, WhileHeld::NeverHeld, None, &never);
    let seqs = |run: &Run| {
        let picket = run.active.picket.as_ref().expect("a picket");
        (
            picket.corvette_think_seq,
            picket.raider_think_seq,
            run.linkdead.get(&CARRIER).map(|state| state.mind_think_seq),
        )
    };
    let before = seqs(&run);
    // Her crew is back while the bubble's batch is out: her answer is struck where it
    // stands. (The tick's hook also drops her record; it is kept here to show that the
    // strike alone is enough to stop the answer.)
    host.forget(Owner::Bubble(run.active.domain), Thinker::Backup, CARRIER);
    run = drive(&run, &host, w, 1, 104, WhileHeld::NeverHeld, None, &never);
    let after = seqs(&run);
    assert_eq!(after.0, before.0 + 1, "the corvette's answer landed");
    assert_eq!(after.1, before.1 + 1, "the raider's answer landed");
    assert_eq!(after.2, before.2, "the struck answer did not");
}

/// **Doc 44 unit MW.11: every think of a bubble runs on a pool worker, never on the thread
/// that steps the bubble**, and one runner serves each seal for the whole encounter.
#[test]
fn mw11_every_think_of_a_bubble_runs_on_a_pool_worker() {
    use crate::space::thinks::test_support::Traced;
    let tw = TestWorld::new();
    let w = tw.world();
    let (mut active, linkdead) = picket_encounter();
    // Doc 47 unit CO.2: her mind answers the raider under every posture now, and inside ten
    // steps her beam takes the raider out of the fight, which then thinks no more. This
    // test is about where thinks run, not the fight: she carries nothing to answer with.
    if let Some(ship) = active.bubble.ships.get_mut("hull.10") {
        ship.lasers.clear();
        ship.railguns.clear();
        ship.torpedo_tubes.clear();
    }
    active.tape = BubbleInputTape::new(&active.bubble);
    active.snapshots = vec![active.bubble.snapshot()];
    let digests = digests_of(&active);
    let host = ThinkHost::on_pool(MindPool::new(3));
    assert_eq!(host.workers(), 3);
    let view = Arc::new(crate::space::sortie::rig::view(ObjId(1)));
    let mut traced = Vec::new();
    for digest in &digests {
        let _ = host.job(None, ObjId(1), *digest, 0, &view, &[]);
        let inner = host.kept_runner(digest).expect("the doctrine has a runner");
        let runner = Arc::new(Traced {
            inner,
            threads: std::sync::Mutex::new(Vec::new()),
        });
        traced.push(Arc::clone(&runner));
        host.install_runner(*digest, runner);
    }
    let fresh = Run {
        active,
        linkdead,
        log: Vec::new(),
    };
    let never = |_: u32| 0_u32;
    let run = drive(
        &fresh,
        &host,
        w,
        10,
        100,
        WhileHeld::NeverHeld,
        None,
        &never,
    );
    // The tenth step's thinks are still out: nothing has asked for them. Wait for the
    // workers before the traces are read.
    let owed = run
        .active
        .thinks_owed
        .expect("the tenth step's thinks are out");
    let (owner, mark) = run.active.think_owner(owed);
    let until = Instant::now() + Duration::from_secs(60);
    while !host.is_answered(owner, mark) {
        assert!(Instant::now() < until, "the last batch never answered");
        std::thread::yield_now();
    }
    let here = std::thread::current()
        .name()
        .unwrap_or("unnamed")
        .to_owned();
    let mut thinks = 0_usize;
    for runner in &traced {
        let threads = runner.threads.lock().expect("the trace");
        for thread in threads.iter() {
            assert!(
                thread.starts_with("tacsim-mind-") && *thread != here,
                "a think ran on {thread}"
            );
        }
        thinks += threads.len();
    }
    // Three minds, a think each for each of the ten steps.
    assert_eq!(thinks, 30, "one think a step for each mind");
    assert_eq!(host.stats().thinks, 30);
    assert_eq!(
        host.stats().runners_built,
        3,
        "one runner for each of the three seals, for {} steps",
        run.active.steps
    );
}

/// **Doc 44 unit MW.11: a tape replay still runs no think.** What the minds decided is on the
/// tape as the commands the applier wrote when their answers landed, so replaying the tape
/// from its first snapshot reproduces the bubble, and it does so with no host, no pool and
/// no runner in reach: [`BubbleInputTape::replay`] takes none.
#[test]
fn mw11_a_replay_of_the_tape_reproduces_the_bubble_and_runs_no_think() {
    let tw = TestWorld::new();
    let w = tw.world();
    // The runner alone, so the host's own rules change no relation off the tape.
    let (mut active, linkdead) = interdiction_encounter();
    active.bubble.ships.remove("hull.20");
    active.carriers.remove(&ESCORT);
    active.hull = RUNNER;
    active.carrier_ship_id = "hull.21".to_owned();
    active.tape = BubbleInputTape::new(&active.bubble);
    let host = ThinkHost::on_pool(MindPool::new(2));
    let mut run = Run {
        active,
        linkdead,
        log: Vec::new(),
    };
    let paused = std::collections::BTreeSet::new();
    let orders = mush_sim::tick::StandingOrderBook::default();
    for n in 0..8_i64 {
        let tick = BubbleTick {
            frozen: false,
            wall_now_s: 101 + n,
            ic_now_s: 1_000 + (101 + n) * 24,
            paused_hulls: &paused,
            standing_orders: &orders,
            host: &host,
            store: None,
            w,
        };
        let mut out = BubbleTickOut::default();
        assert!(step_bubble(
            &mut run.active,
            &tick,
            &mut run.linkdead,
            &mut out
        ));
    }
    assert!(
        mind_commands(&run.active.tape.entries) >= 2,
        "the interdictor's decisions are on the tape: {:?}",
        run.active.tape.entries
    );
    let thinks = host.stats().thinks;
    let (replayed, _) = run.active.tape.replay();
    assert_eq!(host.stats().thinks, thinks, "the replay thought nothing");
    assert_eq!(
        replayed.state_hash(),
        run.active.bubble.state_hash(),
        "the tape alone reproduces the bubble"
    );
}

/// The link-loss record of a hull whose backup mind has her helm.
fn mind_has_helm(hull: ObjId) -> SpaceLinkdeadState {
    SpaceLinkdeadState {
        hull,
        grace_started_wall_s: 1,
        grace_started_ic_s: 1,
        pdc_held: true,
        grace_expired: true,
        queued_return_port: None,
        mind_memory: Vec::new(),
        mind_last_trace: None,
        mind_contacts: super::super::view::ContactBook::default(),
        mind_think_seq: 0,
        backup_act: super::super::backup::BackupAct::Standby,
        return_retry_at_wall_s: 0,
        next_think_at_wall_s: 0,
    }
}

/// **A script a player might seal aboard**: it heaves to as hard as her drive gives when it
/// is told to, and stands otherwise. It asks no lock, and at every think it asks a shot of
/// every mount it reads, which no seat of a sealed mind lets through. Its trace words are
/// its own, so a record says whether it or the house default answered.
const KEEPER: &str = r#"fn think(v, c) {
    stock::rules();
    let s = space(v);
    if s == () { return; }
    for w in v.weapons { fire(w.id, 1); }
    if s.stop == "hailed" && s.stop_by != () {
        let by = space_contact(v, s.stop_by);
        if by != () {
            let dx = by.vx_cms - s.vx_cms;
            let dy = by.vy_cms - s.vy_cms;
            let dz = by.vz_cms - s.vz_cms;
            if isqrt(dx * dx + dy * dy + dz * dz) > 0 {
                helm(course_of(dx, dy, dz));
                speed(max(s.accel_cms2, 1));
                trace("KEEPER_HEAVE_TO");
                return;
            }
        }
    }
    hold();
    trace("KEEPER_STAND");
}"#;

/// **Doc 44 unit MW.13 (Q-44-26; second review, row 17): a sealed mind's rule program is
/// bound, and a mind whose rule program differs behaves differently.**
///
/// Two hulls cross an interdictor with nobody at their helms. Each carries a mind its holder
/// sealed aboard: the same script, sealed through the one seal entry over a Class II core,
/// the runner's with a `LOCK NEAREST` row in its rule program and the escort's with none.
/// Each think runs on its seal's own class row and program, on the one host. The runner
/// comes to hold the interdictor locked; the escort holds no lock; neither fires. Flown by
/// the house default the same two hulls lock nothing, and their records carry its words.
/// (The runner carries the rule because the escort, nearly at rest, is stood down within a
/// step or two, and a hull that is stood down holds nobody hostile.)
///
/// Before the unit a space think was handed an empty program whatever its seal held, and no
/// seat let a hull flown for an absent crew lock.
#[test]
fn mw13_two_sealed_minds_of_one_script_differ_by_their_rule_programs() {
    use crate::arena::conditions::{LockSelector, Rule};
    use crate::arena::core::{ClassTable, CoreClass};
    use crate::arena::schema::AuthoredRule;
    use crate::space::thinks::{SealedMind, test_support::sealed_draft};

    let tw = TestWorld::new();
    let never = |_: u32| 0_u32;
    let start = || {
        let (mut active, mut linkdead) = interdiction_encounter();
        // Each hull carries a mount, so a shot a mind asked would have something to fire.
        for hull in [ESCORT, RUNNER] {
            if let Some(ship) = active.bubble.ships.get_mut(&format!("hull.{}", hull.0)) {
                ship.lasers
                    .push(super::super::weapons::LaserMount::standard_turret(
                        "laser_1",
                    ));
            }
        }
        active.tape = BubbleInputTape::new(&active.bubble);
        active.snapshots = vec![active.bubble.snapshot()];
        linkdead.insert(ESCORT, mind_has_helm(ESCORT));
        linkdead.insert(RUNNER, mind_has_helm(RUNNER));
        Run {
            active,
            linkdead,
            log: Vec::new(),
        }
    };
    let fly = |run: &Run| {
        drive(
            run,
            &ThinkHost::on_pool(MindPool::new(2)),
            tw.world(),
            10,
            100,
            WhileHeld::NeverHeld,
            None,
            &never,
        )
    };
    let lock_of = |run: &Run, hull: ObjId| {
        run.active
            .bubble
            .ships
            .get(&format!("hull.{}", hull.0))
            .and_then(|ship| ship.locked_target_id.clone())
    };
    let trace_of = |run: &Run, hull: ObjId| {
        run.linkdead
            .get(&hull)
            .and_then(|state| state.mind_last_trace.clone())
            .unwrap_or_default()
    };
    let fired = |run: &Run| {
        run.log.iter().any(|entry| match &entry.command {
            BubbleInputCommand::FireOffensiveLasers { shooter_id, .. }
            | BubbleInputCommand::FireRailgun { shooter_id, .. }
            | BubbleInputCommand::FireTorpedoSalvo { shooter_id, .. } => {
                shooter_id.starts_with("hull.")
            }
            _ => false,
        })
    };

    // The house default: no lock, and its own words.
    let house = fly(&start());
    assert_eq!(lock_of(&house, ESCORT), None);
    assert_eq!(lock_of(&house, RUNNER), None);
    assert!(trace_of(&house, ESCORT).starts_with("BACKUP_"));
    assert!(trace_of(&house, RUNNER).starts_with("BACKUP_"));

    // The same hulls, each with the same script sealed aboard; only the rule programs differ.
    let ruled = sealed_draft(
        RUNNER,
        vec![AuthoredRule {
            rule: Rule::lock_rule(LockSelector::Nearest, None, Vec::new()),
            think: Vec::new(),
        }],
        KEEPER,
    );
    let bare = sealed_draft(ESCORT, Vec::new(), KEEPER);
    assert_eq!(ruled.source_sha256, bare.source_sha256, "one script");
    assert_eq!(
        (ruled.class, ruled.core_class, ruled.governed),
        (CoreClass::II, CoreClass::II, false),
        "the class is the hull's core's, under the mind's rating"
    );
    assert!(
        !ruled.channels.contains(&crate::arena::state::Channel::Fire),
        "a ship's seal carries the helm and the sensors"
    );
    let ruled = Arc::new(SealedMind::of(&ruled, KEEPER, &ClassTable::STOCK).expect("it thinks"));
    let bare = Arc::new(SealedMind::of(&bare, KEEPER, &ClassTable::STOCK).expect("it thinks"));
    assert_eq!(ruled.program().rows.len(), 1, "the seal's own program");
    assert!(!ruled.program().rows[0].inert, "its lock row is live");
    assert!(bare.program().rows.is_empty());
    assert_ne!(ruled.program().digest, bare.program().digest);
    assert_eq!(
        *ruled.profile(),
        ClassTable::STOCK.row(CoreClass::II),
        "its thinks are held to its class row, not the doctrine's Class I"
    );
    let _aboard = crate::port::ship_mind::test_support::seal_aboard(vec![
        (RUNNER, Arc::clone(&ruled)),
        (ESCORT, Arc::clone(&bare)),
    ]);
    let sealed = fly(&start());
    assert!(
        trace_of(&sealed, ESCORT).starts_with("KEEPER_")
            && trace_of(&sealed, RUNNER).starts_with("KEEPER_"),
        "the sealed script answered for both: {:?}, {:?}",
        trace_of(&sealed, ESCORT),
        trace_of(&sealed, RUNNER)
    );
    assert_eq!(
        lock_of(&sealed, RUNNER).as_deref(),
        Some(sealed.active.interdictor_ship_id.as_str()),
        "the rule program's LOCK NEAREST row locked the interdictor"
    );
    assert_eq!(
        lock_of(&sealed, ESCORT),
        None,
        "the same script with no rule program locks nothing"
    );
    // A row with a think term acts only when the think sampled that term for this program:
    // the program the think was handed is the seal's own. One hostile is in the runner's
    // view, so `HOSTILES >= 1` holds and `HOSTILES >= 9` does not.
    drop(_aboard);
    let gated = |at_least: u8| {
        let seal = sealed_draft(
            RUNNER,
            vec![AuthoredRule {
                rule: Rule::lock_rule(LockSelector::Nearest, None, Vec::new()),
                think: vec![crate::arena::schema::ThinkPredicate::Hostiles { at_least }],
            }],
            KEEPER,
        );
        let mind = Arc::new(SealedMind::of(&seal, KEEPER, &ClassTable::STOCK).expect("it thinks"));
        let _aboard = crate::port::ship_mind::test_support::seal_aboard(vec![
            (RUNNER, mind),
            (ESCORT, Arc::clone(&bare)),
        ]);
        fly(&start())
    };
    let met = gated(1);
    assert_eq!(
        lock_of(&met, RUNNER).as_deref(),
        Some(met.active.interdictor_ship_id.as_str()),
        "a think term the think sampled true lets the row lock"
    );
    assert_eq!(
        lock_of(&gated(9), RUNNER),
        None,
        "and one it sampled false does not"
    );
    assert!(
        !fired(&sealed) && !fired(&house),
        "neither hull fired, though the sealed script asked a shot at every think and the          runner held a lock"
    );
}

/// A world in which `hull` is a real object whose own record holds `posture`: the record
/// the backup seat reads (`crate::posture::record`).
fn world_with_posture(hull: ObjId, posture: crate::posture::Posture) -> TestWorld {
    use mush_core::{Attr, GOD, ObjKind};
    let mut tw = TestWorld::new();
    let w = tw.world_mut();
    for _ in 0..64 {
        if w.get(hull).is_some() {
            break;
        }
        let _ = w.create(ObjKind::Thing, "Hull", GOD, None);
    }
    assert!(w.get(hull).is_some(), "the rig's hull is an object");
    if !posture.is_return() {
        w.set_attr(
            hull,
            crate::posture::record::POSTURE_ATTR,
            Attr::json(serde_json::json!({ "posture": posture.as_str() }), GOD),
        )
        .expect("the record is written");
    }
    assert_eq!(crate::posture::record::of(tw.world(), hull), posture);
    tw
}

/// **Doc 45 unit AP.9, as doc 47 unit CO.2 rules it (R-47-3): on the bubble's own step, the
/// house default answers the raider that fired on her under every posture; she fires on
/// nobody before she is fired on, and never on the corvette that stopped her.** (Until
/// CO.2 she answered on `HOLD` and `GUARD` and not on `RETURN`.)
///
/// The picket encounter of doc 44 unit MW.11: one Dray whose crew has no link, the customs
/// corvette and the belt raider, which her party holds hostile from the opening: three
/// parties. Her mind heaves her to, customs clear her, the raider lights and opens on her.
/// The three runs differ in the one word on her hull's record. The whole path is flown: the
/// view narrowed to what she may answer, the doctrine's own lock and shot, the seat's gate,
/// the bubble's record of the arrival.
#[test]
fn co2_the_house_default_answers_the_raider_that_fired_on_her_under_every_posture() {
    use crate::posture::Posture;

    for posture in [Posture::Return, Posture::Hold, Posture::Guard] {
        let (beams, lock, on_raider, on_corvette, act, raider) =
            fly_the_picket(posture, None, |_| {});
        assert!(!beams.is_empty(), "{posture}: she fires");
        assert!(
            beams.iter().all(|target| *target == raider),
            "{posture}: on the raider and on nobody else: {beams:?}"
        );
        assert_eq!(lock.as_deref(), Some(raider.as_str()), "{posture}");
        assert!(on_raider, "{posture}: her fire arrives on the raider");
        assert!(!on_corvette, "{posture}: never on the corvette");
        assert_eq!(
            act,
            Some(super::super::backup::BackupAct::ReturningFire),
            "{posture}"
        );
    }
}

/// What [`fly_the_picket`] reads at the end: her beams' targets, her lock, whether her fire
/// arrived on the raider and on the corvette, the seat's last act, and the raider's ship id.
type PicketReading = (
    Vec<String>,
    Option<String>,
    bool,
    bool,
    Option<super::super::backup::BackupAct>,
    String,
);

/// **The picket encounter flown until the raider's fire has arrived on her, and ten steps
/// more**, her hull's record on `posture`, a mind sealed aboard her if `sealed` names one,
/// `each` called on the run after every step until the arrival. Asserts that she locked and
/// fired on nobody before she was fired on.
fn fly_the_picket(
    posture: crate::posture::Posture,
    sealed: Option<Arc<crate::space::thinks::SealedMind>>,
    mut each: impl FnMut(&Run),
) -> PicketReading {
    let _aboard =
        sealed.map(|mind| crate::port::ship_mind::test_support::seal_aboard(vec![(CARRIER, mind)]));
    {
        let tw = world_with_posture(CARRIER, posture);
        let (active, linkdead) = picket_encounter();
        let raider = active
            .picket
            .as_ref()
            .map(|picket| picket.raider_ship_id.clone())
            .expect("a picket");
        let corvette = active
            .picket
            .as_ref()
            .map(|picket| picket.corvette_ship_id.clone())
            .expect("a picket");
        let mut run = Run {
            active,
            linkdead,
            log: Vec::new(),
        };
        let host = ThinkHost::inline();
        let never = |_: u32| 0_u32;
        let arrived_on = |run: &Run, target: &str, shooter: &str| {
            run.active
                .bubble
                .fired_on_by(target)
                .is_some_and(|by| by.contains(shooter))
        };
        let beams = |run: &Run| -> Vec<String> {
            run.log
                .iter()
                .filter_map(|entry| match &entry.command {
                    BubbleInputCommand::FireOffensiveLasers {
                        shooter_id,
                        target_id,
                    } if shooter_id == "hull.10" => Some(target_id.clone()),
                    _ => None,
                })
                .collect()
        };
        // Until the raider's fire arrives on her.
        let mut wall = 100;
        let mut fired_first = false;
        for _ in 0..400 {
            if arrived_on(&run, "hull.10", &raider) {
                break;
            }
            run = drive(
                &run,
                &host,
                tw.world(),
                1,
                wall,
                WhileHeld::NeverHeld,
                None,
                &never,
            );
            wall += 1;
            each(&run);
            if !arrived_on(&run, "hull.10", &raider)
                && (!beams(&run).is_empty()
                    || run
                        .active
                        .bubble
                        .ships
                        .get("hull.10")
                        .is_some_and(|ship| ship.locked_target_id.is_some()))
            {
                fired_first = true;
            }
        }
        assert!(
            arrived_on(&run, "hull.10", &raider),
            "{posture}: the raider's fire arrives on her"
        );
        assert!(
            !fired_first,
            "{posture}: she locked and fired on nobody before she was fired on"
        );
        // Ten steps more.
        let run = drive(
            &run,
            &host,
            tw.world(),
            10,
            wall,
            WhileHeld::NeverHeld,
            None,
            &never,
        );
        let lock = run
            .active
            .bubble
            .ships
            .get("hull.10")
            .and_then(|ship| ship.locked_target_id.clone());
        let trace = run.linkdead.get(&CARRIER).map(|state| state.backup_act);
        (
            beams(&run),
            lock,
            arrived_on(&run, &raider, "hull.10"),
            arrived_on(&run, &corvette, "hull.10"),
            trace,
            raider,
        )
    }
}

/// **Doc 47 unit CO.2 (R-47-2): a ship with a mind sealed aboard answers fire by the house
/// default, and her sealed mind keeps her helm and her sensors.**
///
/// The same picket, the same three parties, with [`KEEPER`] sealed aboard her: it heaves her
/// to by its own burn, asks no lock, and asks a shot of every mount at every think, which
/// its seat drops (a ship's seal carries no fire channel). Under every posture, once the
/// raider's fire has arrived on her, her beams go at the raider and nobody else; before it
/// they go nowhere; her lock is the sealed mind's, which is none, because the house
/// default's answer is fired at its ship without moving her lock; and every word on her
/// record is the sealed mind's. Until CO.2 nothing answered for a hull with a sealed mind.
#[test]
fn co2_a_sealed_minds_ship_answers_by_the_house_default_and_keeps_its_sensors() {
    use crate::arena::core::ClassTable;
    use crate::posture::Posture;
    use crate::space::thinks::{SealedMind, test_support::sealed_draft};

    let seal = sealed_draft(CARRIER, Vec::new(), KEEPER);
    let mind = Arc::new(SealedMind::of(&seal, KEEPER, &ClassTable::STOCK).expect("it thinks"));
    for posture in [Posture::Return, Posture::Hold, Posture::Guard] {
        let mut words = Vec::new();
        let (beams, lock, on_raider, on_corvette, act, raider) =
            fly_the_picket(posture, Some(Arc::clone(&mind)), |run| {
                if let Some(word) = run
                    .linkdead
                    .get(&CARRIER)
                    .and_then(|state| state.mind_last_trace.clone())
                {
                    words.push(word);
                }
            });
        assert!(
            !words.is_empty() && words.iter().all(|word| word.starts_with("KEEPER_")),
            "{posture}: the sealed mind flew her: {words:?}"
        );
        assert!(!beams.is_empty(), "{posture}: the house default answers");
        assert!(
            beams.iter().all(|target| *target == raider),
            "{posture}: on the raider and on nobody else: {beams:?}"
        );
        assert!(on_raider && !on_corvette, "{posture}");
        assert_eq!(
            lock, None,
            "{posture}: her lock is the sealed mind's, and it locked nothing"
        );
        assert_eq!(
            act,
            Some(super::super::backup::BackupAct::ReturningFire),
            "{posture}"
        );
    }
}

/// **Doc 47 unit CO.2 (R-47-2): beside a mind sealed aboard, only the house default's fire
/// channel lands.** One landing, by hand: the raider's fire has arrived on her and the
/// corvette's hail holds her, so a burn against her drift is one the seat lets through. The
/// sealed mind asks her drive cut; the house default asks a burn against her drift, a lock
/// on the raider and a shot. Her drive stays cut, her lock does not move, and her beam goes
/// at the raider. The same house think landed alone, as her first source, burns and locks.
#[test]
fn co2_beside_a_sealed_mind_the_house_default_lands_its_shots_and_nothing_else() {
    use crate::arena::core::ClassTable;
    use crate::arena::intents::{IntentKind, MotorGoal};
    use crate::arena::mind::ThinkResult;
    use crate::posture::Posture;
    use crate::space::thinks::{Answer, SealedMind, Thinker, test_support::sealed_draft};

    let tw = world_with_posture(CARRIER, Posture::Return);
    let seal = sealed_draft(CARRIER, Vec::new(), KEEPER);
    let mind = Arc::new(SealedMind::of(&seal, KEEPER, &ClassTable::STOCK).expect("it thinks"));
    let land = |sealed: bool| {
        let (mut active, mut linkdead) = picket_encounter();
        let raider = active
            .picket
            .as_ref()
            .map(|picket| picket.raider_ship_id.clone())
            .expect("a picket");
        active
            .bubble
            .fired_on
            .entry("hull.10".to_owned())
            .or_default()
            .insert(raider.clone());
        let laser = active
            .bubble
            .ships
            .get("hull.10")
            .and_then(|ship| ship.lasers.first())
            .map(|mount| mount.id.clone())
            .expect("the Dray carries a beam");
        let contact = crate::arena::state::ContactId(7);
        let house = ThinkResult {
            intents: vec![
                IntentKind::Heading {
                    mdeg: super::super::view::helm_word_of([-1_000, 0, 0]),
                },
                IntentKind::Speed { cms: 50 },
                IntentKind::Lock { contact },
                IntentKind::Fire {
                    weapon: laser.as_str().into(),
                    slot: Some(1),
                },
            ],
            ..ThinkResult::default()
        };
        let kept = ThinkResult {
            intents: vec![IntentKind::Goal(MotorGoal::Hold)],
            ..ThinkResult::default()
        };
        let state = linkdead.get_mut(&CARRIER).expect("her record");
        let answer = Answer {
            who: Thinker::Backup,
            unit: CARRIER,
            think_seq: state.mind_think_seq,
            contacts: BTreeMap::from([(contact, raider.clone())]),
            result: Some(if sealed { kept } else { house.clone() }),
            sealed: sealed.then(|| Arc::clone(&mind)),
            house: sealed.then_some(house),
        };
        let _ = super::super::backup::land_in_bubble(
            tw.world(),
            &mut active,
            "hull.10",
            state,
            answer,
            1_000,
        );
        let ship = active.bubble.ships.get("hull.10").expect("her ship");
        let beams: Vec<String> = active
            .tape
            .entries
            .iter()
            .filter_map(|entry| match &entry.command {
                BubbleInputCommand::FireOffensiveLasers { target_id, .. } => {
                    Some(target_id.clone())
                }
                _ => None,
            })
            .collect();
        (
            ship.thrust_mmps2,
            ship.locked_target_id.clone(),
            beams,
            raider,
        )
    };

    let (thrust, lock, beams, raider) = land(true);
    assert_eq!(
        thrust,
        [0, 0, 0],
        "the sealed mind's helm: her drive is cut"
    );
    assert_eq!(lock, None, "her lock is the sealed mind's");
    assert_eq!(beams, vec![raider.clone()], "the house default's shot");

    let (thrust, lock, beams, _) = land(false);
    assert_ne!(
        thrust,
        [0, 0, 0],
        "the same think as her first source burns against her drift"
    );
    assert_eq!(lock.as_deref(), Some(raider.as_str()));
    assert_eq!(beams, vec![raider]);
}

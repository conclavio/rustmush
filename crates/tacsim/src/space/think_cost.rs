//! **What a space think costs the world thread** (doc 44 unit MW.11, Q-44-28): a by-hand
//! measurement lane, `#[ignore]`d. Run it in the release profile:
//!
//! ```text
//! cargo test --release -p tacsim --lib space::picket::think_cost -- --ignored --nocapture
//! ```
//!
//! **The method**, the same before the unit and after it. One fixed bubble for each actor,
//! stepped once so its sensors hold their contacts; then `WARM` thinks thrown away and
//! `SAMPLES` thinks timed with `std::time::Instant`, each carrying the memory the last one
//! left. The calling thread stands for the world thread: what is timed there is what the
//! space tick's thread spends for one think of one actor. Intents are not applied, so every
//! sample thinks on the same snapshot.
//!
//! Before the unit the world thread ran the whole think (`run_space_rhai_think_with_seal`:
//! the view, a runner and a job rebuilt, the pool's round trip, the answer), and the lane
//! timed that call and its stages. Since the unit the world thread *collects* (the view,
//! the job on the kept runner, the hand-over to the pool) and later *lands* (asks the host
//! whether the batch is done, and takes the answers); the think itself runs on a worker.
//! The lane times those two on the calling thread, on a host that holds late steps as the
//! server's does, and times the think apart as the worker runs it. The time the lane spends
//! spinning until a batch is done is not world-thread time and is not counted: the server's
//! tick comes back a second later.
//!
//! The figures of both runs, on Linux and on Windows, are in `docs/design/44-cut-notes.md`
//! under MW.11.

#![cfg(test)]

use std::time::Instant;

use super::*;
use crate::minds::MindPool;
use crate::minds::content::cast::CastActor;
use crate::space::bubble::BubbleInputCommand;
use crate::space::thinks::{Due, Owed, Owner};

/// Thinks thrown away before the timing starts (the compile cache, the lazies, the pool).
const WARM: usize = 200;
/// Thinks timed for each row.
const SAMPLES: usize = 5_000;

/// Nanoseconds since `from`.
fn nanos(from: Instant) -> u64 {
    u64::try_from(from.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

/// One row of the table: the mean, the median, the 99th percentile and the worst, in
/// microseconds to one decimal.
fn row(label: &str, mut samples: Vec<u64>) -> String {
    samples.sort_unstable();
    let n = samples.len().max(1);
    let at = |permille: usize| {
        samples
            .get((n.saturating_sub(1)).saturating_mul(permille) / 1_000)
            .copied()
            .unwrap_or(0)
    };
    let mean = samples.iter().sum::<u64>() / u64::try_from(n).unwrap_or(1);
    let us = |nanos: u64| format!("{}.{}", nanos / 1_000, (nanos % 1_000) / 100);
    format!(
        "{label:<52} mean {:>8}  p50 {:>8}  p99 {:>8}  max {:>9}  (us, n={n})",
        us(mean),
        us(at(500)),
        us(at(990)),
        us(at(1_000)),
    )
}

/// A picket bubble: the corvette and the raider the cast names, and `carriers` Drays
/// closing on the corvette, stepped one second.
fn picket_bubble(carriers: u32) -> (LocalSpaceBubble, KirkwoodPicketState) {
    let mut bubble = LocalSpaceBubble::new(
        "bubble.cost.kirkwood",
        "domain.7",
        "Kirkwood Customs Picket Bubble",
        1_000_000,
    );
    for n in 0..carriers {
        let id = format!("hull.{}", 10 + n);
        bubble.insert_ship(SpaceShipState::new_dray(
            &id,
            "Carrier",
            CARRIER_PARTY,
            [0, i64::from(n) * 900, 0],
            [25_000, 0, 0],
        ));
    }
    let picket = build_kirkwood_picket_ships(&mut bubble, mush_sim::DomainId(7), 1, "hull.10")
        .expect("the picket's ships build");
    let mut tape = BubbleInputTape::new(&bubble);
    let _ = tape.record_and_apply(
        &mut bubble,
        BubbleInputCommand::StepWall {
            dt_wall_ms: 1_000,
            ic_now_s: 1_000_024,
        },
    );
    (bubble, picket)
}

/// The carriers of [`picket_bubble`], as the picket steps by them.
fn carrier_entries(carriers: u32) -> Vec<(ObjId, String, PartyId)> {
    (0..carriers)
        .map(|n| (ObjId(10 + n), format!("hull.{}", 10 + n), CARRIER_PARTY))
        .collect()
}

/// An interdiction bubble: the interdictor, dark and at rest, and two hostile Drays.
fn interdictor_bubble() -> (LocalSpaceBubble, String) {
    let mut bubble = LocalSpaceBubble::new("bubble.cost.int", "domain.5", "Test", 1_000);
    let id = "interdictor.5.1".to_owned();
    let mut interdictor = SpaceShipState::new_corvette(
        &id,
        "Interdictor Picket",
        super::super::tick::INTERDICTOR_PARTY,
        [0, 0, 0],
        [0, 0, 0],
    );
    super::super::custody::apply_space_unit_overlay(
        &mut interdictor,
        &super::super::custody::space_unit_overlay_for_id("tacsim.unit.corvette_interdictor"),
    );
    interdictor.emcon = EmconPosture::Ghost;
    bubble.insert_ship(interdictor);
    for n in 0..2_u32 {
        bubble.insert_ship(SpaceShipState::new_dray(
            format!("hull.{}", 20 + n),
            "Carrier",
            CARRIER_PARTY,
            [-12_000, i64::from(n) * 2_000, 0],
            [25_000, 0, 0],
        ));
    }
    bubble.set_party_relation(
        CARRIER_PARTY,
        super::super::tick::INTERDICTOR_PARTY,
        Some(Mark::Hostile),
    );
    bubble.set_party_relation(
        super::super::tick::INTERDICTOR_PARTY,
        CARRIER_PARTY,
        Some(Mark::Hostile),
    );
    let mut tape = BubbleInputTape::new(&bubble);
    let _ = tape.record_and_apply(
        &mut bubble,
        BubbleInputCommand::StepWall {
            dt_wall_ms: 1_000,
            ic_now_s: 1_024,
        },
    );
    (bubble, id)
}

/// A drone in its bay, as a launch leaves it (doc 44 unit MW.12): its bubble and its seal.
fn drone_in_its_bay() -> (
    crate::space::sortie::DroneFlight,
    Option<crate::arena::mind::MindSeal>,
) {
    (
        crate::space::sortie::rig::flight(1, 8),
        pack_cast_seal(CastActor::Drone, ObjId(77)),
    )
}

/// One think of that drone, collected on the one view.
fn drone_ask(
    host: &ThinkHost,
    drone: &(
        crate::space::sortie::DroneFlight,
        Option<crate::arena::mind::MindSeal>,
    ),
    seq: u64,
    memory: &[u8],
) -> Ask {
    collect_drone_think(
        host,
        None,
        ObjId(77),
        drone.1.as_ref(),
        &drone.0.bubble,
        &mut ContactBook::default(),
        1_000,
        seq,
        memory,
    )
    .expect("the drone's think is collected")
}

/// A host on a pool of two workers that holds late steps, as the server's does.
fn server_host() -> ThinkHost {
    let host = ThinkHost::on_pool(MindPool::new(2));
    host.hold_late_steps();
    host
}

/// **Take `owner`'s answers once its batch is done**, timing only the call that takes
/// them: the calls that find the batch still out are the world thread's too, but each is
/// one lock and one read and they are timed apart ([`held_cost`]).
fn land(host: &ThinkHost, owner: Owner) -> (Vec<Answer>, u64) {
    loop {
        let started = Instant::now();
        match host.settle(owner, (0, 0), Due::default()) {
            Owed::Held => std::thread::yield_now(),
            Owed::Nothing => panic!("a batch was in flight"),
            Owed::Ready { answers, .. } => return (answers, nanos(started)),
        }
    }
}

/// **One actor in a bubble, measured**: the collect and the landing as the world thread
/// runs them, the think as a worker runs it, and the view apart.
fn measure_bubble_actor(
    label: &str,
    bubble: &LocalSpaceBubble,
    who: Thinker,
    ship_id: &str,
    unit: ObjId,
    digest: [u8; 32],
    situation: &Situation,
) -> Vec<String> {
    let mut out = Vec::new();
    let host = server_host();
    let owner = Owner::Bubble(mush_sim::DomainId(1));

    let mut book = ContactBook::default();
    let mut memory: Vec<u8> = Vec::new();
    let mut collect_ns = Vec::with_capacity(SAMPLES);
    let mut land_ns = Vec::with_capacity(SAMPLES);
    let mut think_ns = Vec::with_capacity(SAMPLES);
    for n in 0..WARM + SAMPLES {
        let seq = u64::try_from(n).unwrap_or(0);
        let started = Instant::now();
        let ask = collect_bubble_think(
            &host,
            None,
            bubble,
            who,
            ship_id,
            unit,
            &[digest],
            seq,
            &memory,
            &mut book,
            situation,
        )
        .expect("the think is collected");
        // The same job, for the worker's own cost below.
        let job = ask.jobs.first().expect("one job").clone();
        assert!(host.submit(owner, (0, 0), vec![ask]));
        let collected = nanos(started);

        let (mut answers, landed) = land(&host, owner);
        let result = answers
            .pop()
            .and_then(|answer| answer.result)
            .expect("a clean think");
        assert!(result.fault.is_none(), "{label}: {:?}", result.fault);
        memory = result.memory;

        let started = Instant::now();
        let again = job.think();
        let thought = nanos(started);
        assert!(again.result.fault.is_none());

        if n >= WARM {
            collect_ns.push(collected);
            land_ns.push(landed);
            think_ns.push(thought);
        }
    }
    let whole: Vec<u64> = collect_ns
        .iter()
        .zip(&land_ns)
        .map(|(collected, landed)| collected.saturating_add(*landed))
        .collect();
    out.push(row(
        &format!("{label}: world thread, collect and land"),
        whole,
    ));
    out.push(row(
        &format!("{label}:   collect (view, job, hand-over)"),
        collect_ns,
    ));
    out.push(row(&format!("{label}:   land (take the answers)"), land_ns));

    // The view alone.
    let mut book = ContactBook::default();
    let mut view_ns = Vec::with_capacity(SAMPLES);
    for n in 0..WARM + SAMPLES {
        let started = Instant::now();
        let built =
            super::super::view::space_mind_view(bubble, ship_id, unit, &mut book, situation);
        let took = nanos(started);
        assert!(built.is_some());
        if n >= WARM {
            view_ns.push(took);
        }
    }
    out.push(row(&format!("{label}:   the view"), view_ns));
    out.push(row(
        &format!("{label}: a worker, the think itself"),
        think_ns,
    ));
    assert_eq!(
        host.stats().runners_built,
        1,
        "one runner for {} thinks of one seal",
        WARM + SAMPLES
    );
    out
}

/// What a tick pays to find a batch still out: `settle` on a host that holds, with the
/// worker parked.
fn held_cost() -> String {
    use crate::space::thinks::test_support::{Gate, Gated};
    let host = server_host();
    let digest = crate::minds::content::drone_mining_digest().0;
    let owner = Owner::Cluster(ObjId(1));
    let ask = drone_ask(&host, &drone_in_its_bay(), 0, &[]);
    let inner = host.kept_runner(&digest).expect("the runner is kept");
    let gate = std::sync::Arc::new(Gate::default());
    host.install_runner(
        digest,
        std::sync::Arc::new(Gated {
            inner,
            gate: std::sync::Arc::clone(&gate),
        }),
    );
    drop(ask);
    let ask = drone_ask(&host, &drone_in_its_bay(), 0, &[]);
    assert!(host.submit(owner, (0, 0), vec![ask]));
    let mut held_ns = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let started = Instant::now();
        let owed = host.settle(owner, (0, 0), Due::default());
        held_ns.push(nanos(started));
        assert!(matches!(owed, Owed::Held));
    }
    gate.open();
    let _ = land(&host, owner);
    row("a held step: asking a batch that is still out", held_ns)
}

/// **Doc 44 unit MW.11: the think's cost on the world thread, for each actor and for a
/// step** (the file head has the method). Prints the table; asserts only that every think
/// ran clean and that one runner served each seal.
#[test]
#[ignore = "a by-hand measurement lane (doc 44 unit MW.11); run in release with --nocapture"]
fn mw11_what_a_space_think_costs_the_world_thread() {
    let mut table: Vec<String> = Vec::new();

    // The customs corvette and the raider, with one carrier and with six.
    for carriers in [1_u32, 6] {
        let (bubble, picket) = picket_bubble(carriers);
        let entries = carrier_entries(carriers);
        let situation = picket_situation(&bubble, &picket, &entries, 1_000);
        let corvette = picket.corvette_seal.clone().expect("the corvette's seal");
        table.extend(measure_bubble_actor(
            &format!("corvette, {carriers} carrier(s)"),
            &bubble,
            Thinker::Corvette,
            &picket.corvette_ship_id,
            picket.corvette_hull,
            corvette.source_sha256,
            &situation,
        ));
        let raider = picket.raider_seal.clone().expect("the raider's seal");
        table.extend(measure_bubble_actor(
            &format!("raider, {carriers} carrier(s)"),
            &bubble,
            Thinker::Raider,
            &picket.raider_ship_id,
            picket.raider_hull,
            raider.source_sha256,
            &situation,
        ));
        if carriers == 1 {
            // A carrier whose crew has no link, flown by the house backup mind.
            table.extend(measure_bubble_actor(
                "backup mind, on a carrier",
                &bubble,
                Thinker::Backup,
                "hull.10",
                ObjId(10),
                crate::minds::content::linkdead_return_digest().0,
                &situation,
            ));
        }
    }

    // The interdictor, with two carriers.
    let (bubble, interdictor_id) = interdictor_bubble();
    let seal = pack_cast_seal(CastActor::Interdictor, ObjId(900)).expect("the interdictor's seal");
    let mut situation = Situation::open(1_000);
    situation.stopper = Some(interdictor_id.clone());
    table.extend(measure_bubble_actor(
        "interdictor, 2 carriers",
        &bubble,
        Thinker::Interdictor,
        &interdictor_id,
        ObjId(900),
        seal.source_sha256,
        &situation,
    ));

    // The drone.
    {
        let host = server_host();
        let owner = Owner::Cluster(ObjId(1));
        let mut memory: Vec<u8> = Vec::new();
        let mut collect_ns = Vec::with_capacity(SAMPLES);
        let mut land_ns = Vec::with_capacity(SAMPLES);
        let drone = drone_in_its_bay();
        for n in 0..WARM + SAMPLES {
            let seq = u64::try_from(n).unwrap_or(0);
            let started = Instant::now();
            let ask = drone_ask(&host, &drone, seq, &memory);
            assert!(host.submit(owner, (0, 0), vec![ask]));
            let collected = nanos(started);
            let (mut answers, landed) = land(&host, owner);
            let result = answers
                .pop()
                .and_then(|answer| answer.result)
                .expect("a clean think");
            memory = result.memory;
            if n >= WARM {
                collect_ns.push(collected);
                land_ns.push(landed);
            }
        }
        let whole: Vec<u64> = collect_ns
            .iter()
            .zip(&land_ns)
            .map(|(collected, landed)| collected.saturating_add(*landed))
            .collect();
        table.push(row("drone: world thread, collect and land", whole));
        table.push(row("drone:   collect (view, job, hand-over)", collect_ns));
        table.push(row("drone:   land (take the answers)", land_ns));
    }

    table.push(held_cost());

    // A picket bubble's step, whole: the physics, and what its two minds cost the world
    // thread (the landing at the head of the step, and the collect at its tail).
    for carriers in [1_u32, 6] {
        let (mut bubble, mut picket) = picket_bubble(carriers);
        let entries = carrier_entries(carriers);
        let mut tape = BubbleInputTape::new(&bubble);
        let host = server_host();
        let owner = Owner::Bubble(mush_sim::DomainId(7));
        let mut physics = Vec::new();
        let mut minds = Vec::new();
        let mut owed = false;
        for n in 0..600_usize {
            let mut spent = 0_u64;
            if owed {
                let (answers, landed) = land(&host, owner);
                let started = Instant::now();
                let mut events = Vec::new();
                for answer in answers {
                    deliver_picket_think(&mut bubble, &mut tape, &mut picket, answer, &mut events);
                }
                spent = landed.saturating_add(nanos(started));
            }
            let started = Instant::now();
            let _ = tape.record_and_apply(
                &mut bubble,
                BubbleInputCommand::StepWall {
                    dt_wall_ms: 1_000,
                    ic_now_s: 1_000_025 + i64::try_from(n).unwrap_or(0),
                },
            );
            let stepped = nanos(started);
            let started = Instant::now();
            let asks =
                collect_picket_thinks(&mut bubble, &mut picket, &entries, &host, None, 1_000);
            owed = host.submit(owner, (0, 0), asks);
            spent = spent.saturating_add(nanos(started));
            if n >= 100 {
                physics.push(stepped);
                minds.push(spent);
            }
        }
        table.push(row(
            &format!("picket step, {carriers} carrier(s): the bubble's physics"),
            physics,
        ));
        table.push(row(
            &format!("picket step, {carriers} carrier(s): the minds, world thread"),
            minds,
        ));
    }

    eprintln!(
        "\nMW.11 think cost, after ({} profile)",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    );
    for line in table {
        eprintln!("{line}");
    }
}

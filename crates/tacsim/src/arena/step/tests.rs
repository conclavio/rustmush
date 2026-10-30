#![cfg(test)]

use super::*;
use crate::arena::intents::{Control, ControlState};
use crate::arena::ledger::LedgerEvent;
use crate::arena::state::{Channel, PartyId};
use crate::arena::test_support::{arena_at_epoch, pose, tank};
use crate::config::{ArenaConfig, MatchConfig};

const A: ObjId = ObjId(4);

fn arena() -> Arena {
    arena_at_epoch(vec![tank(A, PartyId::RED, pose(3_500, 3_500))], 1)
}

fn startup() -> Intent {
    Intent {
        unit: A,
        channel: Channel::Helm,
        epoch: 1,
        kind: IntentKind::Startup,
        seq: 1,
    }
}

/// Everything the arena has journalled: the chunks it handed out as it ran,
/// plus whatever is still open when the assertion asks.
#[derive(Default)]
struct Log {
    bytes: Vec<u8>,
}

impl Log {
    fn absorb(&mut self, out: &StepOutput) {
        for chunk in &out.ledger_chunks {
            self.bytes.extend_from_slice(&chunk.bytes);
        }
    }

    fn events(&mut self, arena: &mut Arena) -> Vec<LedgerEvent> {
        arena.ledger.flush();
        for chunk in arena.ledger.take_chunks() {
            self.bytes.extend_from_slice(&chunk.bytes);
        }
        String::from_utf8(self.bytes.clone())
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str::<LedgerEvent>(line).ok())
            .collect()
    }
}

#[test]
fn parallel_pilot_frames_match_serial_bytes_and_consumption() {
    fn check_cohort(pilot_count: u32) {
        let units = (0_u32..pilot_count)
            .map(|i| {
                tank(
                    ObjId(10 + i),
                    PartyId::RED,
                    pose(
                        1_500 + i64::from(i % 6) * 1_000,
                        1_500 + i64::from(i / 6) * 1_000,
                    ),
                )
            })
            .collect();
        let mut serial = arena_at_epoch(units, 1);
        // Reverse seat order: publication still sorts by pilot, independent of chunks.
        serial.pilots.reverse();
        serial.stage_notice(ObjId(10), FrameNotice::PowerOnline);
        let baseline = serial.snapshot_at_release(ObjId(10)).unwrap();
        serial.pending_catchup.insert(
            ObjId(1000),
            serial.catchup_since(ObjId(10), &baseline, None),
        );
        let initial = serial.clone();
        let expected = serial.frames(Tempo::Engaged);
        let wire = |frames: &[(ObjId, Frame)]| {
            frames
                .iter()
                .map(|(pilot, frame)| {
                    (
                        *pilot,
                        serde_json::to_vec(&crate::match_::cockpit::status_frame(
                            crate::arena_task::MatchId(1),
                            frame,
                        ))
                        .unwrap(),
                    )
                })
                .collect::<Vec<_>>()
        };
        for workers in [0, 2, 4, 100] {
            let mut parallel = initial.clone();
            parallel.set_frame_workers(workers);
            let actual = parallel.frames(Tempo::Engaged);
            assert_eq!(wire(&actual), wire(&expected));
            // Include fields omitted from the public projection and same_as.
            assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
            assert_eq!(
                format!("{:?}", parallel.last_frames),
                format!("{:?}", serial.last_frames)
            );
            assert!(parallel.pending_catchup.is_empty());
            assert_eq!(state_hash(&parallel), state_hash(&serial));
            assert!(parallel.frames(Tempo::Engaged).is_empty());
            assert_eq!(parallel.pending_notices, serial.pending_notices);
        }
    }
    for pilot_count in [1, 4, 8, 12, 24] {
        check_cohort(pilot_count);
    }
}

#[test]
fn the_opening_pulse_establishes_the_wall_reference_and_integrates_nothing() {
    let mut arena = arena();
    let out = arena.step(1_000_000, &[], &[]);
    assert_eq!(out.substeps, 0);
    assert_eq!(arena.now_ms, 0);
    assert!(!out.lagged);
    assert!(out.ended.is_none());
    assert_eq!(out.tempo, Tempo::Quiet);
    // The next step is asked for at the period, at the first summary, or at
    // the first sensor pass, whichever is soonest. **16b cut F.3** makes that
    // the pass: every unit has one. **16b cut U.1**: staggered at
    // `stagger_ms(seed, id, pass_ms)` — unit #4's is 482 ms at seed 2,350 —
    // and the wake is the first substep boundary at or after it, 500 on
    // this fixture's 250 ms lattice (16c Q25).
    let pass = crate::arena::rng::stagger_ms(arena.seed, A, 1_000);
    assert_eq!(pass, 482, "the fixture's precondition");
    assert_eq!(arena.substep_len_ms(), 250);
    assert_eq!(out.next_due_ms, 1_000_500);
    assert!(arena.substep_hashes.is_empty());
    // One frame, because the pilot has never been told anything.
    assert_eq!(out.frames.len(), 1);
    assert_eq!(
        out.frames.first().map(|(pilot, _)| *pilot),
        Some(ObjId(1000))
    );
    // And nothing more until something changes.
    let again = arena.step(1_000_000, &[], &[]);
    assert!(again.frames.is_empty());
}

/// An arena at arena millisecond `now` whose heap holds nothing but `due`,
/// for [`Arena::next_due_ms`]'s rule on its own. `Quiet`, so the period is
/// 5,000 ms; the fixture tank's 1,000 cm/s on 1,000 cm cells cuts a 250 ms
/// substep.
fn at_now_with(now: i64, due: Option<i64>) -> Arena {
    let mut arena = arena();
    arena.due.retain(|_| false);
    arena.now_ms = now;
    if let Some(t_ms) = due {
        arena.due.schedule(t_ms, DueKind::PositionSummary);
    }
    assert_eq!(arena.clock.tempo(), Tempo::Quiet);
    assert_eq!(arena.substep_len_ms(), 250, "the fixture's lattice");
    arena
}

/// **16b cut U.1** (16c `T-22`, Q25): the wake is on the substep lattice in
/// absolute arena time — a due event wakes the arena at the first substep
/// boundary at or after it, the period term is the next period multiple, and
/// a late arena is pulled back onto the lattice.
#[test]
fn next_due_is_the_first_lattice_point_at_or_after_the_due_event() {
    const WALL: i64 = 1_000_000;
    // From `now` on the lattice, a due event 3 ms ahead is the next boundary.
    assert_eq!(
        at_now_with(1_000, Some(1_003)).next_due_ms(WALL),
        WALL + 250
    );
    // One a substep and a millisecond ahead is the boundary after that.
    assert_eq!(
        at_now_with(1_000, Some(1_251)).next_due_ms(WALL),
        WALL + 500
    );
    // One exactly on a boundary is that boundary.
    assert_eq!(
        at_now_with(1_000, Some(1_250)).next_due_ms(WALL),
        WALL + 250
    );
    // One due now — or already past — is the next boundary, never now.
    assert_eq!(
        at_now_with(1_000, Some(1_000)).next_due_ms(WALL),
        WALL + 250
    );
    assert_eq!(at_now_with(1_000, Some(400)).next_due_ms(WALL), WALL + 250);
    // An empty heap is the next period multiple.
    assert_eq!(at_now_with(1_000, None).next_due_ms(WALL), WALL + 4_000);
    // The period term wins when it is the nearer.
    assert_eq!(
        at_now_with(4_900, Some(6_000)).next_due_ms(WALL),
        WALL + 100
    );
    // **The pull-back**: a wake that landed late, at 253, with a due event
    // at 500 asks for 500 — 247 ms — and not for 250 after the late wake.
    assert_eq!(at_now_with(253, Some(500)).next_due_ms(WALL), WALL + 247);
    assert_eq!(at_now_with(253, None).next_due_ms(WALL), WALL + 4_747);
}

/// **16b cut U.1**: a period shorter than the substep answers the period,
/// and the answer is never sooner than a millisecond away, wherever `now`
/// and the due event sit.
#[test]
fn next_due_takes_a_short_period_and_is_never_below_a_millisecond() {
    const WALL: i64 = 7_000;
    let mut arena = at_now_with(1_000, Some(3_000));
    arena.arena_cfg.tempo.quiet_ms = 100;
    assert_eq!(arena.next_due_ms(WALL), WALL + 100);
    for now in [0_i64, 1, 249, 250, 251, 999, 4_999, 5_000] {
        for due in [None, Some(0), Some(now), Some(now + 1), Some(now + 250)] {
            let answer = at_now_with(now, due).next_due_ms(WALL);
            assert!(
                answer > WALL,
                "now {now}, due {due:?}: {answer} is not a millisecond away"
            );
            // And every target is a lattice point.
            assert_eq!((now + answer - WALL) % 250, 0, "now {now}, due {due:?}");
        }
    }
}

#[test]
fn a_pulse_runs_the_substeps_the_clamped_dt_is_cut_into() {
    let mut arena = arena();
    arena.step(0, &[], &[]);
    // 1,000 cm cells and a 1,000 cm/s tank: the substep is the 250 ms cap.
    let out = arena.step(1_000, &[], &[]);
    assert_eq!(out.substeps, 4);
    assert_eq!(arena.now_ms, 1_000);
    assert_eq!(arena.substep_hashes.len(), 4);
    // Every substep's hash is recorded, and they differ because `now_ms` does.
    let unique: std::collections::BTreeSet<u64> = arena.substep_hashes.iter().copied().collect();
    assert_eq!(unique.len(), 4);
}

#[test]
fn startup_is_queued_in_phase_two_and_resolved_in_phase_four() {
    let mut arena = arena();
    let out = arena.step(0, &[], &[startup()]);
    assert!(arena.pending.is_empty(), "phase 4 drained the queue");
    assert_eq!(arena.unit(A).map(|unit| unit.power), Some(Power::Starting));
    assert!(out.ended.is_none());
    // `match.startup_ms` later, at the substep boundary at or after it.
    let mut wall = 0_i64;
    for _ in 0..3 {
        wall += 5_000;
        arena.step(wall, &[], &[]);
    }
    assert_eq!(arena.now_ms, 15_000);
    assert_eq!(arena.unit(A).map(|unit| unit.power), Some(Power::Online));
}

#[test]
fn a_stop_ends_the_match_at_phase_one_and_a_further_step_does_nothing() {
    for (stop, reason, word) in [
        (StopReason::Abort, EndReason::Abort, "aborted"),
        (StopReason::Timeout, EndReason::Timeout, "match_ended"),
        (StopReason::Void, EndReason::Void, "void"),
    ] {
        let mut arena = arena();
        let mut journal = Log::default();
        journal.absorb(&arena.step(0, &[], &[]));
        arena.stop(stop);
        let out = arena.step(5_000, &[], &[]);
        journal.absorb(&out);
        assert_eq!(out.ended.map(|end| end.reason), Some(reason));
        assert_eq!(out.substeps, 0);
        assert_eq!(arena.now_ms, 0, "a stop never advances arena time");
        let log = journal.events(&mut arena);
        assert_eq!(log.last().map(|event| event.kind.as_str()), Some(word));
        // And the arena is finished with.
        let after = arena.step(10_000, &[], &[]);
        assert_eq!(after.ended.map(|end| end.reason), Some(reason));
        assert!(after.ledger_chunks.is_empty());
        assert!(after.frames.is_empty());
        assert_eq!(after.substeps, 0);
    }
}

#[test]
fn the_duration_bound_ends_the_match_with_a_timeout() {
    let mut arena = Arena::new(
        crate::arena::test_support::custody(
            vec![tank(A, PartyId::RED, pose(3_500, 3_500))],
            Vec::new(),
        ),
        &ArenaConfig::default(),
        &MatchConfig {
            max_duration_ms: 3_000,
            ..MatchConfig::default()
        },
        &crate::config::SensorsConfig::default(),
    )
    .expect("a three-second match");
    let mut journal = Log::default();
    journal.absorb(&arena.step(0, &[], &[]));
    let out = arena.step(2_000, &[], &[]);
    journal.absorb(&out);
    assert!(out.ended.is_none());
    let out = arena.step(4_000, &[], &[]);
    journal.absorb(&out);
    // **16b cut M.2** (16c `T-01`): at the deadline's own millisecond, 3,000,
    // and not at the end of the pulse that crossed it (4,000 before the cut).
    assert_eq!(
        out.ended,
        Some(MatchEnd {
            reason: EndReason::Timeout,
            t_ms: 3_000
        })
    );
    // The pulse stopped at the substep the deadline fell in: 3,000 is the end
    // of the fourth of this pulse's eight, and the other four never ran.
    assert_eq!(out.substeps, 4);
    let log = journal.events(&mut arena);
    assert_eq!(
        log.last().map(|event| event.kind.as_str()),
        Some("match_ended")
    );
    // And a further step does nothing at all.
    let after = arena.step(6_000, &[], &[]);
    assert_eq!(after.substeps, 0);
    assert!(after.ledger_chunks.is_empty());
    assert_eq!(arena.now_ms, 3_000);
}

#[test]
fn a_released_seat_in_the_same_burst_purges_the_order_it_arrived_with() {
    // Doc 16 §3.1's burst, end to end through `step`: phase 1 raises the
    // epoch, phase 2 refuses the order that was typed under the old one.
    let mut arena = arena();
    let out = arena.step(
        0,
        &[Control {
            unit: A,
            channel: Channel::Helm,
            epoch: 2,
            state: ControlState::Released,
            pilot: None,
        }],
        &[startup()],
    );
    assert!(out.ended.is_none());
    assert_eq!(arena.unit(A).map(|unit| unit.power), Some(Power::Off));
    assert!(arena.pending.is_empty());
    // **16b cut N.1** amends doc 16 §3.8's *the tank idles*: a released seat
    // hands the tank to a machine in `Withdraw` (doc 07 §1.1's fallback
    // controller) — **17b cut M4.3**: a mind, seeded `Withdraw` — so the
    // schedule now carries its first think beside the position summary.
    // Nothing else about the burst changed.
    assert_eq!(
        arena.unit(A).map(|unit| unit.controller),
        Some(crate::arena::state::Controller::Mind)
    );
    assert_eq!(
        arena.machine_state(A),
        Some(crate::arena::doctrine::MachineState::Withdraw)
    );
    // 16b cut F.3 adds the third: this platform's own sensor pass. **16b cut
    // M.2** (16c `T-01`) the fourth: the duration bound.
    assert_eq!(
        arena.due.len(),
        4,
        "the position summary, the duration bound, the released tank's first think, its pass"
    );
    let mut journal = Log::default();
    journal.absorb(&out);
    let log = journal.events(&mut arena);
    // 17b cut M3.6: and the opening's one `mind_sealed` per unit (was
    // `["match_started"]`).
    assert_eq!(
        log.iter()
            .map(|event| event.kind.as_str())
            .filter(|kind| *kind != "mind_sealed")
            .collect::<Vec<_>>(),
        vec!["match_started"]
    );
    assert_eq!(
        log.iter()
            .filter(|event| event.kind.as_str() == "mind_sealed")
            .count(),
        arena.units.len()
    );
}

#[test]
fn a_fire_intent_raises_the_alert_tempo_and_journals_the_change() {
    let mut arena = arena();
    arena.step(0, &[], &[]);
    let fire = Intent {
        unit: A,
        channel: Channel::Fire,
        epoch: 1,
        kind: IntentKind::Fire {
            weapon: smol_str::SmolStr::new("cannon"),
            slot: None,
        },
        seq: 2,
    };
    let out = arena.step(5_000, &[], &[fire]);
    // A queued fire intent is an `Alert` trigger and an accepted **shot** is an
    // `Engaged` one (doc 16 §3.1's table), and this fixture's tank carries no
    // weapon: the order is queued, phase 4 refuses it `NoWeapon`, and the tempo
    // stops at the trigger the queue itself is.
    assert_eq!(out.tempo, Tempo::Alert);
    let mut journal = Log::default();
    journal.absorb(&out);
    let log = journal.events(&mut arena);
    assert!(
        log.iter().any(|event| event.kind
            == EventKind::Tempo {
                from: Tempo::Quiet,
                to: Tempo::Alert
            }),
        "the tempo change was not journalled: {log:?}"
    );
}

#[test]
fn the_position_summary_keeps_its_cadence_in_arena_time() {
    let mut arena = arena();
    arena.step(0, &[], &[]);
    let mut wall = 0_i64;
    for _ in 0..4 {
        wall += 5_000;
        arena.step(wall, &[], &[]);
    }
    assert_eq!(arena.now_ms, 20_000);
    let mut journal = Log::default();
    let log = journal.events(&mut arena);
    let summaries: Vec<i64> = log
        .iter()
        .filter(|event| event.kind.as_str() == "position_summary")
        .map(|event| event.t_ms)
        .collect();
    assert_eq!(summaries, vec![5_000, 10_000, 15_000, 20_000]);
    // The next one is scheduled, always. The four above are already journalled,
    // so it is the *schedule* and not the log that says the cadence continues —
    // and `next_t_ms` cannot say it since 16b cut F.3 gives every unit a sensor
    // pass, which on this fixture falls nearer (unit #4's, at 20,000 ms plus
    // its stagger; 16b cut U.1 made that `stagger_ms`, 482 ms at seed 2,350).
    let offset = crate::arena::rng::stagger_ms(arena.seed, A, 1_000);
    assert!(
        offset > 0,
        "the fixture's precondition: #4's pass is not on a second"
    );
    assert!(
        arena
            .due
            .iter()
            .any(|event| event.t_ms == 25_000 && event.kind == DueKind::PositionSummary),
        "the next position summary is not on the schedule: {:?}",
        arena.due.iter().collect::<Vec<_>>()
    );
    assert_eq!(
        arena.due.next_t_ms(),
        Some(20_000 + offset),
        "unit #4's own stagger"
    );
}

/// **16b cut S.1, the pulse end to end**: the sensor pass is due in phase
/// 3(c), the contact it mints reaches the pilot's frame, the tempo answers to
/// it, and the counters say what it cost.
#[test]
fn a_sensor_pass_runs_in_the_pulse_and_its_contact_reaches_the_frame_and_the_tempo() {
    use crate::arena::test_support::scout;
    const B: ObjId = ObjId(9);
    // Two tanks that see and shoot, four cells apart on the 8 by 8 window.
    // **16b cut E.1** (16c `T-32`): the derived `tempo.alert_range_cm` is the
    // lesser of twice the cannon's 2.5 km and the window's own diagonal, and
    // on an 8 by 8 window of 1,000 cm cells the diagonal is the lesser by a
    // long way — `isqrt(8² + 8²) = 11` cells. A contact 40 m away is still
    // well inside it.
    let mut arena = crate::arena::test_support::arena_at_epoch(
        vec![
            scout(A, PartyId::RED, pose(500, 500)),
            scout(B, PartyId::BLUE, pose(4_500, 500)),
        ],
        1,
    );
    assert_eq!(arena.alert_range_cm, 11_000, "the window's own diagonal");
    // The opening pulse establishes the wall reference and integrates nothing,
    // so no pass has run yet.
    let out = arena.step(0, &[], &[]);
    assert_eq!(out.perception.passes, 0);
    assert!(arena.contacts.is_empty());
    // One second of arena time: both staggered passes (at their seeded
    // offsets inside the first second, `rng::stagger_ms`; 4 ms and 9 ms
    // under `id % pass_ms` before 16b cut U.1) run, each holds the other,
    // and the tempo rises to `Alert`.
    let out = arena.step(1_000, &[], &[]);
    assert_eq!(out.perception.passes, 2, "{:?}", out.perception);
    assert_eq!(out.perception.detected, 2);
    assert_eq!(out.perception.traces, 2);
    // **16b cut J.1**: two 5-cell lines between cells (0,0) and (4,0) of the
    // flat window, three interior cells each and nothing blocking — six. The
    // walk counted the endpoints too and read ten. **16b cut J.2**: the flat
    // 8 by 8 window is one partly filled 16-block with no flag and no
    // elevation, so each of the two lines is proved clear at one block read
    // and no cell is read at all.
    assert_eq!(
        (out.perception.cell_probes, out.perception.horizon_probes),
        (0, 2)
    );
    assert_eq!(out.tempo, Tempo::Alert);
    assert_eq!(arena.contacts.len(), 2);
    // The pilot of A is told about B, at the right bearing and range, and about
    // nothing else.
    let frame = out
        .frames
        .iter()
        .find(|(pilot, _)| *pilot == ObjId(1000))
        .map(|(_, frame)| frame)
        .unwrap_or_else(|| panic!("A's pilot was sent a frame: {:?}", out.frames));
    assert_eq!(frame.contacts.len(), 1);
    let contact = frame
        .contacts
        .first()
        .cloned()
        .unwrap_or_else(|| panic!("one contact"));
    assert_eq!(contact.class, crate::arena::state::Classification::Unknown);
    assert_eq!(contact.bearing_mdeg, 90_000, "B is due east of A");
    assert_eq!(contact.range_cm, 4_000);
    // And it is the id `draw_contact_id` mints for the pair, so a pilot's
    // `LOCK` resolves through `ShadowContacts::resolve`.
    assert_eq!(
        contact.id,
        crate::arena::rng::draw_contact_id(arena.seed, A, B)
    );
    assert!(arena.contacts.resolve(A, contact.id).is_some());
    assert!(
        arena.contacts.resolve(B, contact.id).is_none(),
        "an id minted for A is not B's"
    );
    // Three more seconds carry both past `classify_after_ms`, and the classify
    // draw is in the ledger against the label doc 16 §3.5 pins.
    let mut journal = Log::default();
    let mut wall = 1_000_i64;
    for _ in 0..4 {
        wall += 1_000;
        journal.absorb(&arena.step(wall, &[], &[]));
    }
    let log = journal.events(&mut arena);
    let classified: Vec<&LedgerEvent> = log
        .iter()
        .filter(|event| event.kind.as_str() == "classified")
        .collect();
    assert_eq!(classified.len(), 2, "{:?}", log);
    for event in classified {
        // 16b cut M.3: a list, and a sensor classification makes one draw.
        let [(label, _)] = event.draws.as_slice() else {
            panic!("a classification carries its one draw: {event:?}");
        };
        assert!(label.starts_with("classify:"), "{label}");
    }
}

#[test]
fn a_pulse_past_the_clamp_journals_one_lagged_notice() {
    let mut arena = arena();
    arena.step(0, &[], &[]);
    // Quiet is a 5,000 ms period and `max_catch_up_steps` is 4, so 20,000 ms
    // is the clamp and 20,001 is lag.
    let out = arena.step(30_000, &[], &[]);
    assert!(out.lagged);
    assert_eq!(arena.now_ms, 20_000, "the idle wall time is skipped");
    let mut journal = Log::default();
    journal.absorb(&out);
    let log = journal.events(&mut arena);
    assert!(
        log.iter()
            .any(|event| event.kind == EventKind::Lagged { dt_ms: 20_000 })
    );
}

// -----------------------------------------------------------------------
// 16b cut D.2 — phase 2 speaks (`B-03`)
// -----------------------------------------------------------------------

/// The one-tank fixture, under a name a per-case binding does not shadow.
fn fresh() -> Arena {
    arena()
}

/// Every notice this pulse staged for the fixture's one pilot.
fn told(out: &StepOutput) -> Vec<FrameNotice> {
    out.frames
        .iter()
        .filter(|(pilot, _)| *pilot == ObjId(1000))
        .flat_map(|(_, frame)| frame.notices.iter().cloned())
        .collect()
}

/// The refusals among them, as `(order word, refusal, weapon)`.
fn refusals(out: &StepOutput) -> Vec<(String, Refusal, Option<String>)> {
    told(out)
        .into_iter()
        .filter_map(|notice| match notice {
            FrameNotice::Refused {
                order,
                refusal,
                weapon,
                ..
            } => Some((
                order.to_string(),
                refusal,
                weapon.map(|weapon| weapon.to_string()),
            )),
            _ => None,
        })
        .collect()
}

fn order(unit: ObjId, kind: IntentKind, seq: u64) -> Intent {
    Intent {
        unit,
        channel: kind.channel_required(),
        epoch: 1,
        kind,
        seq,
    }
}

/// **16b cut D.2** (`B-03`): a wreck answers every order, in the order's own
/// word, and the word is [`IntentKind::as_str`]'s in both phases.
#[test]
fn d2_phase_two_names_the_order_it_refused() {
    let mut arena = fresh();
    arena.step(0, &[], &[]);
    if let Some(unit) = arena.unit_mut(A) {
        unit.limits.wreck = true;
    }
    let out = arena.step(
        5_000,
        &[],
        &[
            order(A, IntentKind::Startup, 1),
            order(A, IntentKind::Heading { mdeg: 90_000 }, 2),
            order(A, IntentKind::Speed { cms: 100 }, 3),
            order(
                A,
                IntentKind::Lock {
                    contact: ContactId(7),
                },
                4,
            ),
            order(
                A,
                IntentKind::Fire {
                    weapon: SmolStr::new("cannon"),
                    slot: None,
                },
                5,
            ),
        ],
    );
    assert_eq!(
        refusals(&out),
        vec![
            ("startup".to_owned(), Refusal::Wreck, None),
            ("heading".to_owned(), Refusal::Wreck, None),
            ("speed".to_owned(), Refusal::Wreck, None),
            ("lock".to_owned(), Refusal::Wreck, None),
            ("fire".to_owned(), Refusal::Wreck, Some("cannon".to_owned())),
        ],
        "the five order words are `IntentKind::as_str`'s, and only `fire` names a weapon"
    );
}

/// The other three of the four phase 2 says out loud, each on its own order.
#[test]
fn d2_phase_two_says_the_motion_limit_the_cold_tank_and_the_full_queue() {
    // (a) motion disabled, on a live tank with its power up.
    let mut arena = fresh();
    arena.step(0, &[], &[]);
    if let Some(unit) = arena.unit_mut(A) {
        unit.power = Power::Online;
        unit.limits.motion_disabled = true;
    }
    let out = arena.step(
        5_000,
        &[],
        &[
            order(A, IntentKind::Heading { mdeg: 90_000 }, 1),
            order(A, IntentKind::Speed { cms: 100 }, 2),
        ],
    );
    assert_eq!(
        refusals(&out),
        vec![
            ("heading".to_owned(), Refusal::MotionDisabled, None),
            ("speed".to_owned(), Refusal::MotionDisabled, None),
        ]
    );

    // (b) the cold tank, which is the refusal doc 16 §3.3 names.
    let mut arena = fresh();
    arena.step(0, &[], &[]);
    let out = arena.step(5_000, &[], &[order(A, IntentKind::Heading { mdeg: 0 }, 1)]);
    assert_eq!(
        refusals(&out),
        vec![("heading".to_owned(), Refusal::PowerNotOnline, None)]
    );

    // (c) the ninth discrete order of one burst. `max_pending_per_unit` is
    // eight a seat a unit per arena second (16b cut M.1, `T-82`), and all
    // nine are this seat's orders in arena second 0 (phase 2 runs before the
    // clock moves), so the ninth is refused once however fast the arena is
    // pulsing — which is the whole of why this is staged rather than
    // dropped in silence.
    let mut arena = fresh();
    arena.step(0, &[], &[]);
    let burst: Vec<Intent> = (0..9)
        .map(|n: u64| order(A, IntentKind::Startup, n + 1))
        .collect();
    let out = arena.step(5_000, &[], &burst);
    assert_eq!(
        refusals(&out),
        vec![("startup".to_owned(), Refusal::QueueFull, None)],
        "one refusal for one dropped order"
    );
}

/// **16b cut M.1** (`T-82`): (d) the bound is a rate in ARENA seconds,
/// counted on the seat that gave the order.
///
/// Phase 2 runs at the arena time the previous pulse ended on, so a pulse
/// stepped to wall 500 applies its orders at arena 0, and one stepped to wall
/// 1,500 applies them at arena 1,000.
#[test]
fn m1_the_order_bound_is_a_rate_per_arena_second_counted_on_the_seat() {
    // Eight in arena second 0, then a ninth in a later pulse still inside
    // second 0: refused, although the queue was drained in between.
    let mut arena = fresh();
    arena.step(0, &[], &[]);
    let eight: Vec<Intent> = (0..8)
        .map(|n: u64| order(A, IntentKind::Startup, n + 1))
        .collect();
    let out = arena.step(500, &[], &eight);
    assert!(refusals(&out).is_empty(), "{:?}", refusals(&out));
    assert!(arena.pending.is_empty(), "phase 4 drained the eight");
    let out = arena.step(900, &[], &[order(A, IntentKind::Startup, 9)]);
    assert_eq!(arena.now_ms, 900);
    assert_eq!(
        refusals(&out),
        vec![("startup".to_owned(), Refusal::QueueFull, None)],
        "the ninth order inside one arena second is refused, whatever the pulse rate"
    );

    // The same ninth in a pulse whose phase 2 runs at arena 1,000: accepted.
    let mut arena = fresh();
    arena.step(0, &[], &[]);
    arena.step(500, &[], &eight);
    arena.step(1_000, &[], &[]);
    let out = arena.step(1_500, &[], &[order(A, IntentKind::Startup, 9)]);
    assert!(
        refusals(&out).is_empty(),
        "a new arena second is a new count: {:?}",
        refusals(&out)
    );

    // The machine's own orders are never counted.
    let mut arena = fresh();
    for n in 0..9_u64 {
        assert_eq!(
            apply_intent(
                &mut arena,
                &order(A, IntentKind::Startup, n + 1),
                IntentSource::Machine
            ),
            IntentOutcome::Queued,
            "machine order {n}"
        );
    }

    // A seat's order to a tank a machine drives is still a seat's: a gunner
    // left on a machine-driven hull is bounded like anybody. (17b cut
    // M4.5a: the machine is a mind, handed the hull as a release hands it.)
    let mut arena = fresh();
    arena.set_controller(
        A,
        crate::arena::state::Controller::Mind,
        crate::arena::doctrine::MachineState::Withdraw,
    );
    let outcomes: Vec<IntentOutcome> = (0..9_u64)
        .map(|n| {
            apply_intent(
                &mut arena,
                &order(A, IntentKind::Startup, n + 1),
                IntentSource::Seat,
            )
        })
        .collect();
    assert_eq!(
        outcomes
            .iter()
            .filter(|o| **o == IntentOutcome::Queued)
            .count(),
        8
    );
    assert_eq!(
        outcomes.last(),
        Some(&IntentOutcome::Refused(Refusal::QueueFull)),
        "the ninth seat order on a mind-driven unit is refused"
    );
}

/// **The five phase 2 does not say**, each for its own reason — see
/// [`Arena::phase_two_tells`].
#[test]
fn d2_phase_two_is_silent_where_there_is_nobody_to_tell() {
    // `Epoch`: doc 16 §3.1's `FIRE` then `EXIT` burst, working as written.
    let mut arena = fresh();
    arena.step(0, &[], &[]);
    arena.epochs.raise(A, Channel::Helm, 2);
    let out = arena.step(5_000, &[], &[order(A, IntentKind::Startup, 1)]);
    assert!(refusals(&out).is_empty(), "{:?}", refusals(&out));

    // `Channel`: a routing fault, not a pilot's order.
    let mut arena = fresh();
    arena.step(0, &[], &[]);
    let out = arena.step(
        5_000,
        &[],
        &[Intent {
            unit: A,
            channel: Channel::Fire,
            epoch: 1,
            kind: IntentKind::Startup,
            seq: 1,
        }],
    );
    assert!(refusals(&out).is_empty(), "{:?}", refusals(&out));

    // `Unknown`: there is no unit, so there is no pilot.
    let mut arena = fresh();
    arena.step(0, &[], &[]);
    let out = arena.step(5_000, &[], &[order(ObjId(4_242), IntentKind::Startup, 1)]);
    assert!(refusals(&out).is_empty(), "{:?}", refusals(&out));

    // `Linkdead`: the seat has nobody at it.
    let mut arena = fresh();
    arena.step(0, &[], &[]);
    if let Some(unit) = arena.unit_mut(A) {
        unit.linkdead = true;
    }
    let out = arena.step(5_000, &[], &[order(A, IntentKind::Startup, 1)]);
    assert!(refusals(&out).is_empty(), "{:?}", refusals(&out));

    // `NotMachine`, and every `Order` whatever it is refused for: doctrine is
    // a commander's and a tank's pilots are not its audience.
    let mut arena = fresh();
    arena.step(0, &[], &[]);
    let doctrine = crate::deliberation::order::Order {
        squad: crate::deliberation::squad::SquadId::parse("red-1").expect("a squad id"),
        objective: crate::deliberation::order::Objective::Hold,
        target: None,
        horizon_ms: 60_000,
        constraints: crate::deliberation::order::Constraints::default(),
        ttl_ms: 30_000,
        issued_ms: 1,
        issuer: None,
        by: crate::deliberation::order::CharacterRef::Persona {
            squad: crate::deliberation::squad::SquadId {
                team: crate::arena::state::PartyId::RED,
                index: 1,
            },
        },
        spot: None,
    };
    let out = arena.step(
        5_000,
        &[],
        &[order(A, IntentKind::Order(Box::new(doctrine)), 1)],
    );
    assert!(refusals(&out).is_empty(), "{:?}", refusals(&out));

    // **16b cut Z.2**: and every `ReleaseOrder`, for the same reason. The
    // world fans a lead's `RELEASE` to every unit of the squad's roster, and
    // a roster tank that is a wreck is refused `Wreck` — one of the refusals
    // phase 2 otherwise tells — before anything asks whether a machine
    // flies it; that is nobody's news, so nothing is staged.
    let mut arena = fresh();
    arena.step(0, &[], &[]);
    if let Some(unit) = arena.unit_mut(A) {
        unit.limits.wreck = true;
    }
    let released = order(
        A,
        IntentKind::ReleaseOrder {
            squad: crate::deliberation::squad::SquadId::parse("red-1").expect("a squad id"),
            issuer: Some(A),
        },
        1,
    );
    assert_eq!(
        crate::arena::intents::apply_intent(
            &mut arena.clone(),
            &released,
            crate::arena::intents::IntentSource::Seat
        ),
        IntentOutcome::Refused(Refusal::Wreck),
        "the witness is a release refused for a reason phase 2 tells elsewhere"
    );
    let out = arena.step(5_000, &[], &[released]);
    assert!(refusals(&out).is_empty(), "{:?}", refusals(&out));
}

/// **16b cut D.2**: the pilot's own two limits ride the frame, so the cockpit
/// can refuse an order to a dead tank at once instead of a pulse later.
#[test]
fn d2_the_frame_carries_the_wreck_and_the_motion_limit() {
    let mut arena = fresh();
    let out = arena.step(0, &[], &[]);
    let frame = out
        .frames
        .first()
        .map(|(_, frame)| frame.clone())
        .unwrap_or_else(|| panic!("the opening frame"));
    assert!(!frame.wreck);
    assert!(!frame.immobile);
    // Both are compared, so the flip sends a frame at once.
    let mut dead = frame.clone();
    dead.wreck = true;
    assert!(!frame.same_as(&dead, 1_000));
    let mut stuck = frame.clone();
    stuck.immobile = true;
    assert!(!frame.same_as(&stuck, 1_000));
    if let Some(unit) = arena.unit_mut(A) {
        unit.limits.wreck = true;
        unit.limits.motion_disabled = true;
    }
    let out = arena.step(5_000, &[], &[]);
    let frame = out
        .frames
        .first()
        .map(|(_, frame)| frame.clone())
        .unwrap_or_else(|| panic!("the flip sends a frame"));
    assert!(frame.wreck && frame.immobile);
}

// -----------------------------------------------------------------------
// 16b cut D.2 — one sentence for a round on a spent section (`B-15`)
// -----------------------------------------------------------------------

/// The gunner and the victim of the spent-section cases: two tanks, **nobody
/// seated on either**, because the whole point of the window is that it is
/// written whether or not anybody is there to read it.
fn spent_rig() -> Arena {
    crate::arena::test_support::arena_of(vec![
        tank(A, PartyId::RED, pose(1_500, 1_500)),
        tank(ObjId(9), PartyId::BLUE, pose(1_500, 5_500)),
    ])
}

/// One `hit` line, as the impact journals it.
fn hit(shot_seq: u64, section: &str) -> EventKind {
    EventKind::Hit {
        shot_seq,
        unit: A,
        target: ObjId(9),
        section: SmolStr::new(section),
        facet: SmolStr::new("front"),
        armor_section: SmolStr::new(section),
        rounds: 1,
        damage: 0,
        armor_left: 0,
        internals_left: 0,
        consequences: Vec::new(),
        cause: SmolStr::new(format!("shot:{shot_seq}")),
    }
}

/// Stage the notices one `hit` owes, with `shot_seq` marked spent or not —
/// the same call `Arena::substep`'s staging scan makes, and the scratch
/// filled the way `Arena::apply_hit` fills it in the same substep.
fn stage_hit(arena: &mut Arena, shot_seq: u64, section: &str, spent: bool, t_ms: i64) {
    if spent {
        arena.spent_hits.insert(shot_seq);
    }
    let mut in_flight: Option<(ObjId, ObjId)> = None;
    arena.stage_pilot_notice(t_ms, &hit(shot_seq, section), &mut in_flight, None);
}

/// What the victim has been told since the last drain.
fn victims_notices(arena: &mut Arena) -> Vec<FrameNotice> {
    arena.pending_notices.remove(&ObjId(9)).unwrap_or_default()
}

/// **The scratch never survives the substep it was filled in** (16b cut D.2),
/// which is the whole of why it is not folded into the state hash.
#[test]
fn d2_the_spent_shot_scratch_is_drained_by_the_staging_scan() {
    let mut arena = spent_rig();
    stage_hit(&mut arena, 1, "hull", true, 0);
    assert!(
        arena.spent_hits.is_empty(),
        "the `hit` arm did not drain the scratch: {:?}",
        arena.spent_hits
    );
    // And a round on a live section leaves nothing in it either.
    stage_hit(&mut arena, 2, "hull", false, 1_000);
    assert!(arena.spent_hits.is_empty());
}

/// **The window is arena time, and it is exactly `SPENT_NOTICE_MS`** (16b cut
/// D.2): a second round at 4,999 ms is silent, one at 5,000 ms is said.
#[test]
fn d2_a_spent_section_is_said_once_every_five_seconds() {
    for (second, said) in [(4_999_i64, false), (5_000, true)] {
        let mut arena = spent_rig();
        stage_hit(&mut arena, 1, "hull", true, 0);
        assert_eq!(
            victims_notices(&mut arena),
            vec![FrameNotice::StruckSpent {
                section: SmolStr::new("hull")
            }],
            "the first round on a spent section is always said"
        );
        stage_hit(&mut arena, 2, "hull", true, second);
        let told = victims_notices(&mut arena);
        assert_eq!(
            told.is_empty(),
            !said,
            "a second round at {second} ms: {told:?}"
        );
    }
    // A DIFFERENT section is a different key, so it is said at once.
    let mut arena = spent_rig();
    stage_hit(&mut arena, 1, "hull", true, 0);
    let _ = victims_notices(&mut arena);
    stage_hit(&mut arena, 2, "turret", true, 1_000);
    assert_eq!(
        victims_notices(&mut arena),
        vec![FrameNotice::StruckSpent {
            section: SmolStr::new("turret")
        }]
    );
}

/// **The decision and the write happen with nobody seated** (16b cut D.2),
/// which is what makes the fold rate-independent: a pilot who sits down four
/// seconds into the window reads nothing until it runs out.
#[test]
fn d2_the_spent_window_is_kept_whether_or_not_anybody_is_seated() {
    let mut arena = spent_rig();
    assert!(arena.pilots.is_empty(), "nobody is seated on either tank");
    stage_hit(&mut arena, 1, "hull", true, 0);
    assert_eq!(
        arena
            .spent_said
            .get(&(ObjId(9), ObjId(9), SmolStr::new("hull")))
            .copied(),
        Some(0),
        "the entry was written with no frame to carry it: {:?}",
        arena.spent_said
    );
    // Nothing is keyed on the gunner: he holds no contact, so he read what he
    // always reads and earned no window (`Arena::spent_said`).
    assert!(
        arena
            .spent_said
            .keys()
            .all(|(told, _, _)| *told == ObjId(9)),
        "{:?}",
        arena.spent_said
    );
    arena.pending_notices.clear();
    // Somebody takes the seat four seconds in.
    arena.pilots.push(crate::arena::state::Pilot {
        unit: ObjId(9),
        pilot: ObjId(1_001),
        station: SmolStr::new("cockpit"),
        epoch: 1,
    });
    stage_hit(&mut arena, 2, "hull", true, 4_000);
    assert!(
        victims_notices(&mut arena).is_empty(),
        "the window the empty pod kept was not honoured"
    );
    stage_hit(&mut arena, 3, "hull", true, 5_000);
    assert_eq!(
        victims_notices(&mut arena),
        vec![FrameNotice::StruckSpent {
            section: SmolStr::new("hull")
        }]
    );
}

/// **Entries are pruned on touch and never by a pulse** (16b cut D.2): a
/// per-pulse sweep would make the table — which the state hash folds — a
/// function of the cadence, which is exactly what rate independence refuses.
#[test]
fn d2_the_spent_window_is_pruned_on_touch_and_never_by_a_pulse() {
    let mut arena = spent_rig();
    let stale = (ObjId(4_242), ObjId(4_243), SmolStr::new("turret"));
    arena.spent_said.insert(stale.clone(), 0);
    // Ten seconds of pulses, which is twice the window: the entry stands,
    // because nothing touched the table.
    let mut wall = 0_i64;
    for _ in 0..7 {
        wall += 2_000;
        arena.step(wall, &[], &[]);
    }
    assert!(arena.now_ms >= 10_000);
    assert!(
        arena.spent_said.contains_key(&stale),
        "a pulse swept the table: {:?}",
        arena.spent_said
    );
    // One spent round touches it, and the stale entry goes with that touch.
    let now = arena.now_ms;
    stage_hit(&mut arena, 1, "hull", true, now);
    assert!(
        !arena.spent_said.contains_key(&stale),
        "the touch did not prune: {:?}",
        arena.spent_said
    );
}

/// **A fresh section is unchanged** (16b cut D.2): the round that finds
/// armour still says what it always said, and so does the one that finishes
/// the section.
#[test]
fn d2_a_round_on_a_live_section_says_what_it_always_said() {
    let mut arena = spent_rig();
    stage_hit(&mut arena, 1, "hull", false, 0);
    let told = victims_notices(&mut arena);
    assert_eq!(told.len(), 1);
    assert!(
        matches!(told.first(), Some(FrameNotice::Struck { section, .. }) if section == "hull"),
        "{told:?}"
    );
    assert!(
        arena.spent_said.is_empty(),
        "a live section wrote a window entry: {:?}",
        arena.spent_said
    );
    // And the section's own loss line is untouched by any of this.
    let mut in_flight: Option<(ObjId, ObjId)> = None;
    arena.stage_pilot_notice(
        0,
        &EventKind::SectionDestroyed {
            unit: ObjId(9),
            section: SmolStr::new("hull"),
            // 16b cut D.4 (`T-08`): nothing shot it off.
            by: None,
            consequences: Vec::new(),
        },
        &mut in_flight,
        None,
    );
    let told = victims_notices(&mut arena);
    assert!(
        matches!(
            told.first(),
            Some(FrameNotice::SectionLost { section, .. }) if section == "hull"
        ),
        "{told:?}"
    );
}

/// **The victim's own two lines** (16b cut D.2, `B-03`): staged off the
/// ledger's own events, to the victim and to nobody else.
#[test]
fn d2_the_victim_is_told_when_his_own_tank_goes() {
    let mut arena = spent_rig();
    let mut in_flight: Option<(ObjId, ObjId)> = None;
    arena.stage_pilot_notice(
        0,
        &EventKind::UnitDisabled { unit: ObjId(9) },
        &mut in_flight,
        None,
    );
    assert_eq!(victims_notices(&mut arena), vec![FrameNotice::OwnDisabled]);
    arena.stage_pilot_notice(
        0,
        // 16b cut D.4 (`T-08`): nobody's round did it, so nobody is credited.
        &EventKind::UnitDestroyed {
            unit: ObjId(9),
            by: None,
        },
        &mut in_flight,
        None,
    );
    assert_eq!(victims_notices(&mut arena), vec![FrameNotice::OwnDestroyed]);
    assert!(
        arena.pending_notices.is_empty(),
        "somebody else was told: {:?}",
        arena.pending_notices
    );
    // With the shooter on the event the gunner's kill line comes first, and
    // the victim's own line after it. **16b cut D.4**: off `by` and no longer
    // off the substep's in-flight scratch, which is left empty here on
    // purpose — a stale scratch must not be what credits a kill.
    arena.stage_pilot_notice(
        0,
        &EventKind::UnitDestroyed {
            unit: ObjId(9),
            by: Some(A),
        },
        &mut in_flight,
        None,
    );
    assert_eq!(
        arena.pending_notices.remove(&A).unwrap_or_default(),
        vec![FrameNotice::Kill { contact: None }]
    );
    assert_eq!(victims_notices(&mut arena), vec![FrameNotice::OwnDestroyed]);
}

// -----------------------------------------------------------------------
// 16b cut D.4 — the gunner's arms read the shooter off the event (`T-08`)
// -----------------------------------------------------------------------

/// The gunner holding the victim **classified and read**, which is the state
/// both of `T-08`'s stale readers need: the section word is gated on the
/// classification, and `observe_section_lost` writes into the reading.
fn watching_gunner() -> Arena {
    let mut arena = spent_rig();
    let id = crate::arena::rng::draw_contact_id(arena.seed, A, ObjId(9));
    let plot = crate::arena::state::SeenPose::default();
    arena.contacts.insert(
        A,
        id,
        crate::arena::state::ShadowContact {
            seen_condition: Some(crate::arena::condition::Reading {
                at_ms: 0,
                condition: crate::arena::condition::Condition::default()
                    .with_band(0, crate::arena::condition::ConditionBand::Green),
            }),
            ..crate::arena::state::ShadowContact::sighted(
                ObjId(9),
                Classification::Classified { party: PartyId::BLUE },
                0,
                plot,
            )
        },
    );
    arena
}

/// What the gunner has been told since the last drain.
fn gunners_notices(arena: &mut Arena) -> Vec<FrameNotice> {
    arena.pending_notices.remove(&A).unwrap_or_default()
}

/// **`T-08`**: a `section_destroyed` or a `unit_destroyed` that no round did
/// is credited to nobody, even when a round resolved on the same target
/// earlier in the same substep — and it writes no hashed contact state.
#[test]
fn d4_a_destroy_no_round_did_is_credited_to_nobody() {
    let mut arena = watching_gunner();
    let mut in_flight: Option<(ObjId, ObjId)> = None;

    // The round arrives and resolves. The scratch is set by the impact and
    // cleared by the outcome, which is the whole of `T-08`'s fix.
    arena.stage_pilot_notice(
        0,
        &EventKind::Impact {
            shot_seq: 1,
            unit: A,
            target: ObjId(9),
        },
        &mut in_flight,
        None,
    );
    assert_eq!(in_flight, Some((A, ObjId(9))));
    arena.stage_pilot_notice(0, &hit(1, "hull"), &mut in_flight, None);
    assert_eq!(in_flight, None, "the `hit` did not clear the pair");
    let _ = gunners_notices(&mut arena);
    let _ = victims_notices(&mut arena);
    let settled = state_hash(&arena);

    // Now something that is not that round takes a section off the same
    // target, later in the same substep.
    arena.stage_pilot_notice(
        0,
        &EventKind::SectionDestroyed {
            unit: ObjId(9),
            section: SmolStr::new("hull"),
            by: None,
            consequences: Vec::new(),
        },
        &mut in_flight,
        None,
    );
    assert!(
        gunners_notices(&mut arena).is_empty(),
        "the earlier shooter was credited with a section nobody shot off"
    );
    assert_eq!(
        state_hash(&arena),
        settled,
        "`observe_section_lost` wrote hashed contact state for a round that had resolved"
    );
    // The victim's own line is untouched: it is a fact about their tank.
    assert!(
        matches!(
            victims_notices(&mut arena).first(),
            Some(FrameNotice::SectionLost { section, .. }) if section == "hull"
        ),
        "the victim was not told"
    );

    arena.stage_pilot_notice(
        0,
        &EventKind::UnitDestroyed {
            unit: ObjId(9),
            by: None,
        },
        &mut in_flight,
        None,
    );
    assert!(
        gunners_notices(&mut arena).is_empty(),
        "a kill was credited to a round that had already resolved"
    );
    assert_eq!(victims_notices(&mut arena), vec![FrameNotice::OwnDestroyed]);
}

/// **And with the shooter on the event the two lines are exactly as they
/// were** (16b cut D.4, `T-08`): the fix is about *where* the shooter is read
/// from and not about what is said.
#[test]
fn d4_a_destroy_a_round_did_still_names_the_gunner_and_writes_his_reading() {
    let mut arena = watching_gunner();
    // The scratch is deliberately empty: nothing here depends on it any more.
    let mut in_flight: Option<(ObjId, ObjId)> = None;
    let before = state_hash(&arena);
    arena.stage_pilot_notice(
        0,
        &EventKind::SectionDestroyed {
            unit: ObjId(9),
            section: SmolStr::new("hull"),
            by: Some(A),
            consequences: Vec::new(),
        },
        &mut in_flight,
        None,
    );
    assert!(
        matches!(
            gunners_notices(&mut arena).first(),
            Some(FrameNotice::TargetSectionLost { section, .. }) if section == "hull"
        ),
        "the gunner was told nothing about the section he took off"
    );
    assert_ne!(
        state_hash(&arena),
        before,
        "the gunner's own reading was not written"
    );
    let _ = victims_notices(&mut arena);

    arena.stage_pilot_notice(
        0,
        &EventKind::UnitDestroyed {
            unit: ObjId(9),
            by: Some(A),
        },
        &mut in_flight,
        None,
    );
    assert!(
        matches!(
            gunners_notices(&mut arena).first(),
            Some(FrameNotice::Kill { contact: Some(_) })
        ),
        "the gunner was not told he had finished the tank"
    );
}

/// **The scratch serves `miss` alone now** (16b cut D.4, `T-08`): it is set by
/// the impact, read by the outcome, and `None` afterwards either way.
#[test]
fn d4_the_in_flight_pair_is_cleared_by_a_hit_and_by_a_miss() {
    let mut arena = watching_gunner();
    let mut in_flight: Option<(ObjId, ObjId)> = None;
    arena.stage_pilot_notice(
        0,
        &EventKind::Impact {
            shot_seq: 1,
            unit: A,
            target: ObjId(9),
        },
        &mut in_flight,
        None,
    );
    arena.stage_pilot_notice(
        0,
        &EventKind::Miss {
            shot_seq: 1,
            reason: SmolStr::new("dispersion"),
        },
        &mut in_flight,
        None,
    );
    assert!(
        matches!(
            gunners_notices(&mut arena).first(),
            Some(FrameNotice::Miss { .. })
        ),
        "the miss was not said"
    );
    assert_eq!(in_flight, None, "the `miss` did not clear the pair");

    // And a second `miss` with nothing in flight says nothing at all.
    arena.stage_pilot_notice(
        0,
        &EventKind::Miss {
            shot_seq: 2,
            reason: SmolStr::new("dispersion"),
        },
        &mut in_flight,
        None,
    );
    assert!(
        gunners_notices(&mut arena).is_empty(),
        "a miss with no impact before it named a shooter"
    );
}

/// **16b cut AE.2**: a miss keeps its reason for a target the gunner holds,
/// and carries none for one he does not — `moved`, `blocked` and `cover` are
/// facts about where the target is that his sensors have not earned. `Hit.`
/// on an unheld target is untouched.
#[test]
fn ae2_a_miss_on_a_target_the_gunner_does_not_hold_carries_no_reason() {
    let mut arena = watching_gunner();
    let mut in_flight: Option<(ObjId, ObjId)> = None;
    let impact = |target: ObjId| EventKind::Impact {
        shot_seq: 1,
        unit: A,
        target,
    };
    let miss = || EventKind::Miss {
        shot_seq: 1,
        reason: SmolStr::new(crate::arena::damage::BLOCKED),
    };
    // Held: the reason, as ever.
    arena.stage_pilot_notice(0, &impact(ObjId(9)), &mut in_flight, None);
    arena.stage_pilot_notice(0, &miss(), &mut in_flight, Some((2, 7)));
    assert!(
        matches!(
            gunners_notices(&mut arena).first(),
            Some(FrameNotice::Miss { contact: Some(_), reason, launch_cell: Some((2, 7)) })
                if reason == crate::arena::damage::BLOCKED
        ),
        "a held target's miss keeps its reason"
    );
    // Not held: no contact and no reason. The launch cell remains the
    // gunner's own fact, independent of the target's held classification.
    arena.stage_pilot_notice(0, &impact(ObjId(77)), &mut in_flight, None);
    arena.stage_pilot_notice(0, &miss(), &mut in_flight, Some((2, 7)));
    assert_eq!(
        gunners_notices(&mut arena),
        vec![FrameNotice::Miss {
            contact: None,
            reason: SmolStr::new_static(""),
            launch_cell: Some((2, 7)),
        }],
        "an unheld target's miss says nothing of why"
    );
    // And the unheld hit is the bare `Hit.` it always was.
    arena.stage_pilot_notice(0, &impact(ObjId(77)), &mut in_flight, None);
    arena.stage_pilot_notice(
        0,
        &EventKind::Hit {
            shot_seq: 1,
            unit: A,
            target: ObjId(77),
            section: SmolStr::new("hull"),
            facet: SmolStr::new("front"),
            armor_section: SmolStr::new("hull"),
            rounds: 1,
            damage: 10,
            armor_left: 10,
            internals_left: 10,
            consequences: Vec::new(),
            cause: SmolStr::new("test"),
        },
        &mut in_flight,
        None,
    );
    assert!(matches!(
        gunners_notices(&mut arena).first(),
        Some(FrameNotice::Hit {
            contact: None,
            section: None,
            ..
        })
    ));
}

// -----------------------------------------------------------------------
// 16b cut M.2 — the duration bound, the substep count and the drain
// -----------------------------------------------------------------------

/// Two scouts four cells apart, each holding the other after its first
/// pass, on a match whose duration bound is `max_duration_ms`.
fn m2_pair(max_duration_ms: u32) -> Arena {
    use crate::arena::test_support::scout;
    m2_pair_with(
        max_duration_ms,
        scout(ObjId(9), PartyId::BLUE, pose(4_500, 500)),
    )
}

/// The same with `blue` for the target — which must stand where
/// [`m2_pair`]'s does, four cells east of `A`.
fn m2_pair_with(max_duration_ms: u32, blue: crate::arena::state::CustodyUnit) -> Arena {
    use crate::arena::test_support::scout;
    Arena::new(
        crate::arena::test_support::custody(
            vec![scout(A, PartyId::RED, pose(500, 500)), blue],
            Vec::new(),
        ),
        &ArenaConfig::default(),
        &MatchConfig {
            max_duration_ms,
            ..MatchConfig::default()
        },
        &crate::config::SensorsConfig::default(),
    )
    .expect("two scouts")
}

/// Move the duration bound to `deadline`, as `Arena::new` would have put
/// it: the one `MatchDeadline` on the heap, and the backstop's number.
fn m2_move_deadline(arena: &mut Arena, deadline: i64) {
    let _ = arena.due.retain(|kind| *kind != DueKind::MatchDeadline);
    arena.due.schedule(deadline, DueKind::MatchDeadline);
    arena.match_cfg.max_duration_ms = u32::try_from(deadline).unwrap_or(u32::MAX);
}

/// **16b cut M.2** (16c `T-01`): a deadline that falls inside a substep ends
/// the match at its own millisecond on a pulse whose end is later, and an
/// impact due after it but inside the same substep is **not** applied — no
/// `impact`, no `hit`, no `miss`, the round still in flight and still on the
/// heap — so `match_ended { timeout }` is the last line and a round landing
/// after the whistle cannot turn a timeout into an elimination.
#[test]
fn m2_a_deadline_inside_a_substep_ends_the_match_there_and_nothing_after_it_lands() {
    const B: ObjId = ObjId(9);
    let mut arena = m2_pair(60_000);
    let mut journal = Log::default();
    journal.absorb(&arena.step(0, &[], &[]));
    for wall in [1_000, 2_000, 3_000] {
        journal.absorb(&arena.step(wall, &[], &[]));
    }
    assert_eq!(arena.now_ms, 3_000);
    // A live cannon, locked on the contact its own pass holds.
    if let Some(state) = arena.unit_mut(A) {
        state.power = Power::Online;
    }
    let contact = crate::arena::rng::draw_contact_id(arena.seed, A, B);
    assert_eq!(arena.resolve_lock(A, contact, 3_000), Ok(()));
    let seq = arena
        .resolve_fire(A, "cannon", 3_010)
        .unwrap_or_else(|refusal| panic!("the shot was refused: {refusal:?}"));
    let impact_at = arena.shot(seq).map_or(0, |shot| shot.impact_at_ms);
    assert!(
        impact_at > 3_011 && impact_at <= 3_250,
        "the fixture needs the impact inside the next substep: {impact_at}"
    );
    // The whistle one millisecond before the round arrives.
    let deadline = impact_at - 1;
    m2_move_deadline(&mut arena, deadline);
    let out = arena.step(4_000, &[], &[]);
    journal.absorb(&out);
    assert_eq!(
        out.ended,
        Some(MatchEnd {
            reason: EndReason::Timeout,
            t_ms: deadline
        }),
        "the deadline's own millisecond, not the pulse's end (4,000)"
    );
    // 16c `T-10`: the first of four substeps ended it, so one ran.
    assert_eq!(out.substeps, 1);
    assert_eq!(arena.now_ms, 3_250, "the substep the deadline fell in");
    let log = journal.events(&mut arena);
    for word in ["impact", "hit", "miss"] {
        assert!(
            !log.iter().any(|event| event.kind.as_str() == word),
            "a round due after the deadline journalled `{word}`: {log:?}"
        );
    }
    assert!(arena.shot(seq).is_some(), "the round was resolved");
    assert!(
        arena
            .due
            .iter()
            .any(|event| event.kind == DueKind::Impact { shot_seq: seq }),
        "the impact left the heap"
    );
    assert_eq!(
        log.last().map(|event| (event.t_ms, event.kind.clone())),
        Some((
            deadline,
            EventKind::MatchEnded {
                reason: EndReason::Timeout
            }
        )),
        "the timeout is the last line, at the deadline"
    );
}

/// Run `arena` to 3,000 and have `A` lock `ObjId(9)` and fire its cannon at
/// 3,010, the way every T-01 test below does: the journal so far, the shot's
/// sequence and the millisecond its round is due.
fn m2_fire(arena: &mut Arena) -> (Log, u64, i64) {
    const B: ObjId = ObjId(9);
    let mut journal = Log::default();
    journal.absorb(&arena.step(0, &[], &[]));
    for wall in [1_000, 2_000, 3_000] {
        journal.absorb(&arena.step(wall, &[], &[]));
    }
    if let Some(state) = arena.unit_mut(A) {
        state.power = Power::Online;
    }
    let contact = crate::arena::rng::draw_contact_id(arena.seed, A, B);
    assert_eq!(arena.resolve_lock(A, contact, 3_000), Ok(()));
    let seq = arena
        .resolve_fire(A, "cannon", 3_010)
        .unwrap_or_else(|refusal| panic!("the shot was refused: {refusal:?}"));
    let impact_at = arena.shot(seq).map_or(0, |shot| shot.impact_at_ms);
    (journal, seq, impact_at)
}

/// Two rounds accepted from different cells resolve in reverse order in
/// one substep, after the shooter has moved to a third cell. Each notice
/// retains its own round's acceptance cell after `resolve_impact` removes
/// both shots; neither current position nor latest launch can stand in.
#[test]
fn m5r_overlapping_misses_keep_their_own_launch_cells() {
    let mut arena = m2_pair(60_000);
    let (_, first, _) = m2_fire(&mut arena);
    assert_eq!(
        arena.shot(first).and_then(|shot| shot.from_cell),
        Some((0, 0))
    );
    {
        let shooter = arena.unit_mut(A).expect("gunner");
        shooter.pose = pose(1_500, 500);
        // Give the fixture a second round in flight. Its acceptance still
        // captures the actual firing cell through the production resolver.
        shooter.ready_at_ms.insert(SmolStr::new("cannon"), 0);
    }
    let second = arena.resolve_fire(A, "cannon", 3_020).expect("second shot");
    assert_eq!(
        arena.shot(second).and_then(|shot| shot.from_cell),
        Some((1, 0))
    );
    arena.unit_mut(A).expect("gunner").pose = pose(2_500, 500);

    // Isolate origin bookkeeping from dispersion and damage. Both rounds
    // lose their predictions, and the later shot arrives first.
    let _ = arena
        .due
        .retain(|kind| !matches!(kind, DueKind::Impact { .. }));
    for (seq, at_ms) in [(first, 3_200), (second, 3_100)] {
        let shot = arena.shots.get_mut(&seq).expect("accepted shot");
        shot.aim_cell = None;
        shot.impact_at_ms = at_ms;
        arena.due.schedule(at_ms, DueKind::Impact { shot_seq: seq });
    }
    let _ = gunners_notices(&mut arena);
    assert!(!arena.substep(
        250,
        &mut PerceptionCounters::default(),
        &mut MotorCounters::default(),
    ));
    assert!(arena.shot(first).is_none() && arena.shot(second).is_none());
    let cells: Vec<_> = gunners_notices(&mut arena)
        .into_iter()
        .filter_map(|notice| match notice {
            FrameNotice::Miss { launch_cell, .. } => Some(launch_cell),
            _ => None,
        })
        .collect();
    assert_eq!(cells, vec![Some((1, 0)), Some((0, 0))]);
}

/// Every `match_ended` line in `log`, with its millisecond.
fn m2_ends(log: &[LedgerEvent]) -> Vec<(i64, EventKind)> {
    log.iter()
        .filter(|event| matches!(event.kind, EventKind::MatchEnded { .. }))
        .map(|event| (event.t_ms, event.kind.clone()))
        .collect()
}

/// **The production order**: the match's own `MatchDeadline`, scheduled by
/// `Arena::new` at sequence 2, is on the heap at `impact_at` with a lower
/// sequence than the round's `Impact`, so it pops FIRST and the impact is
/// popped under the horizon it shrank.
fn m2_assert_the_deadline_pops_first(arena: &Arena, seq: u64, impact_at: i64) {
    let deadline = arena
        .due
        .iter()
        .find(|event| event.kind == DueKind::MatchDeadline)
        .map(|event| (event.t_ms, event.seq));
    let impact = arena
        .due
        .iter()
        .find(|event| event.kind == DueKind::Impact { shot_seq: seq })
        .map(|event| (event.t_ms, event.seq));
    assert_eq!(deadline.map(|(t_ms, _)| t_ms), Some(impact_at));
    assert_eq!(impact.map(|(t_ms, _)| t_ms), Some(impact_at));
    assert!(
        deadline.map(|(_, seq)| seq) < impact.map(|(_, seq)| seq),
        "the fixture needs the match's own deadline to pop first: {deadline:?} {impact:?}"
    );
}

/// **16b cut M.2** (16c `T-01`): an impact due at the deadline's **own**
/// millisecond still lands, in the same due loop, before the timeout is
/// settled — the horizon shrinks to the deadline, not below it.
///
/// In the production order: the duration bound is the one `Arena::new`
/// scheduled, so it pops BEFORE the impact on their shared millisecond, and
/// the impact is popped under the horizon the deadline set. A horizon one
/// millisecond short of the deadline, or a loop that stopped at the
/// deadline's own arm, would leave this round unapplied.
#[test]
fn m2_an_impact_at_the_deadlines_own_millisecond_still_lands() {
    // Where the round lands, from a probe with the bound out of the way —
    // the bound decides nothing before it, so the two runs agree to there.
    let (_, _, impact_at) = m2_fire(&mut m2_pair(60_000));
    let bound = u32::try_from(impact_at).unwrap_or(u32::MAX);
    let mut arena = m2_pair(bound);
    let (mut journal, seq, landed_at) = m2_fire(&mut arena);
    assert_eq!(landed_at, impact_at, "the probe and the run agree");
    m2_assert_the_deadline_pops_first(&arena, seq, impact_at);
    let out = arena.step(4_000, &[], &[]);
    journal.absorb(&out);
    let log = journal.events(&mut arena);
    assert!(
        log.iter()
            .any(|event| event.kind.as_str() == "impact" && event.t_ms == impact_at),
        "the round on the deadline's own millisecond did not land: {log:?}"
    );
    assert!(arena.shot(seq).is_none(), "the round is still in flight");
    assert_eq!(
        out.ended.map(|end| (end.reason, end.t_ms)),
        Some((EndReason::Timeout, impact_at)),
        "a round that does not kill leaves the timeout standing"
    );
    assert_eq!(
        m2_ends(&log),
        vec![(
            impact_at,
            EventKind::MatchEnded {
                reason: EndReason::Timeout
            }
        )],
        "one end, the timeout"
    );
}

/// **16b cut M.2** (16c `T-01`): **a kill on the deadline's own millisecond
/// wins.** The same production order as the test above, with a target the
/// round destroys: one hull section, its armour and internals less than one
/// round's damage, facing the shooter, whose loss is `DestroyUnit`. The
/// impact pops after the deadline on their shared millisecond, destroys the
/// last blue unit, and elimination is settled before the timeout is — so the
/// match ends by elimination, and no timeout line is journalled beside it.
#[test]
fn m2_a_kill_on_the_deadlines_own_millisecond_wins() {
    use crate::arena::resolution::{BandProfile, Consequence, SectionProfile, UnitProfile};
    use crate::arena::test_support::scout;
    let glass = || {
        let mut blue = scout(ObjId(9), PartyId::BLUE, pose(4_500, 500));
        // Facing west, down the line of fire, so the round meets the one
        // facet `tactical_unit` has: `hull/front`, ten armour over eight
        // internals against the cannon's 120.
        blue.pose.heading_mdeg = 270_000;
        blue.profile = UnitProfile {
            bands: vec![BandProfile {
                band: SmolStr::new("front"),
                width_cm: 350,
                height_cm: 250,
                sections: vec![(SmolStr::new("hull"), 100)],
            }],
            sections: vec![SectionProfile {
                id: SmolStr::new("hull"),
                facets: vec![SmolStr::new("front")],
                consequences: vec![Consequence::DestroyUnit],
                struck_through: None,
            }],
        };
        blue
    };
    let (_, _, impact_at) = m2_fire(&mut m2_pair_with(60_000, glass()));
    let bound = u32::try_from(impact_at).unwrap_or(u32::MAX);
    let mut arena = m2_pair_with(bound, glass());
    let (mut journal, seq, landed_at) = m2_fire(&mut arena);
    assert_eq!(landed_at, impact_at, "the probe and the run agree");
    m2_assert_the_deadline_pops_first(&arena, seq, impact_at);
    let out = arena.step(4_000, &[], &[]);
    journal.absorb(&out);
    let log = journal.events(&mut arena);
    assert!(
        log.iter()
            .any(|event| event.kind.as_str() == "hit" && event.t_ms == impact_at),
        "the fixture needs the round to hit on the deadline: {log:?}"
    );
    assert!(
        arena.unit(ObjId(9)).is_some_and(|unit| unit.limits.wreck),
        "the round on the deadline did not destroy the target: {log:?}"
    );
    assert_eq!(
        out.ended.map(|end| end.reason),
        Some(EndReason::Elimination {
            winner: Some(PartyId::RED)
        }),
        "the kill on the whistle is an elimination"
    );
    let ends = m2_ends(&log);
    assert_eq!(
        ends.iter()
            .map(|(_, kind)| kind.clone())
            .collect::<Vec<_>>(),
        vec![EventKind::MatchEnded {
            reason: EndReason::Elimination {
                winner: Some(PartyId::RED)
            }
        }],
        "exactly one end, the elimination, and no timeout beside it: {ends:?}"
    );
    assert_eq!(
        log.last().map(|event| event.kind.as_str()),
        Some("match_ended"),
        "nothing after the end"
    );
}

/// **16b cut M.2** (16c `T-01`): an elimination settled in the substep the
/// deadline falls in wins, the order phase 5 always had. The wreck is the
/// fixture's hand, placed so the side goes out in the very substep whose
/// due loop pops the deadline. Every pulse's output is absorbed — the ending
/// pulse flushes the ledger's tail into its own `ledger_chunks` — so the
/// line count below is read off the whole black box.
#[test]
fn m2_an_elimination_in_the_deadlines_own_substep_wins() {
    const B: ObjId = ObjId(9);
    let mut arena = m2_pair(3_100);
    let mut journal = Log::default();
    journal.absorb(&arena.step(0, &[], &[]));
    journal.absorb(&arena.step(3_000, &[], &[]));
    assert!(arena.ended.is_none());
    if let Some(state) = arena.unit_mut(B) {
        state.limits.wreck = true;
    }
    let out = arena.step(4_000, &[], &[]);
    journal.absorb(&out);
    assert_eq!(
        out.ended.map(|end| end.reason),
        Some(EndReason::Elimination {
            winner: Some(PartyId::RED)
        })
    );
    // The elimination is stamped at the end of the substep it was settled
    // in, 3,250 — the deadline's substep — which is `settle_elimination`'s
    // rule from before this cut, not the deadline's millisecond.
    assert_eq!(out.ended.map(|end| end.t_ms), Some(3_250));
    let log = journal.events(&mut arena);
    assert!(!log.is_empty(), "the journal read nothing");
    assert_eq!(
        m2_ends(&log),
        vec![(
            3_250,
            EventKind::MatchEnded {
                reason: EndReason::Elimination {
                    winner: Some(PartyId::RED)
                }
            }
        )],
        "exactly one end, the elimination: {log:?}"
    );
}

/// **16b cut AP.2** (16c `B-87`): the settle order Q.7 set — elimination
/// first, `Spent` second — holds for the disarmed side. Both magazines are
/// emptied and blue's gun shot off by the fixture's hand between two pulses,
/// so the next substep finds both sides dry AND blue with no gun left: that
/// is red's elimination win, never the `Spent` draw it was before this cut
/// (`spent` counts an emptied weapon table as nothing to fire). One end in
/// the black box, at the substep's end.
#[test]
fn ap2_a_side_disarmed_on_a_dry_substep_is_eliminated_not_spent() {
    const B: ObjId = ObjId(9);
    let mut arena = m2_pair(60_000);
    let mut journal = Log::default();
    journal.absorb(&arena.step(0, &[], &[]));
    journal.absorb(&arena.step(1_000, &[], &[]));
    assert!(arena.ended.is_none());
    for id in [A, B] {
        if let Some(state) = arena.unit_mut(id) {
            state.ammo.insert(SmolStr::new("cannon"), 0);
        }
    }
    let mut said: Vec<Notice> = Vec::new();
    arena.apply_consequence(
        B,
        &crate::arena::resolution::Consequence::RemoveWeapons {
            section: SmolStr::new("turret"),
        },
        1_000,
        None,
        &mut said,
    );
    assert!(arena.spent(), "both sides are dry: Q.7's predicate holds");
    let out = arena.step(2_000, &[], &[]);
    journal.absorb(&out);
    assert_eq!(
        out.ended.map(|end| end.reason),
        Some(EndReason::Elimination {
            winner: Some(PartyId::RED)
        }),
        "the side with no gun left is out, settled before Spent"
    );
    assert_eq!(out.ended.map(|end| end.t_ms), Some(1_250));
    let log = journal.events(&mut arena);
    assert_eq!(
        m2_ends(&log),
        vec![(
            1_250,
            EventKind::MatchEnded {
                reason: EndReason::Elimination {
                    winner: Some(PartyId::RED)
                }
            }
        )],
        "exactly one end, the elimination, and no Spent beside it: {log:?}"
    );
}

/// **16b cut M.2** (16c `T-10`): `substeps` is what ran. A pulse cut into
/// four whose second substep ends the match reports two; the opening pulse
/// reports none.
#[test]
fn m2_a_pulse_ended_in_its_second_of_four_substeps_reports_two() {
    let mut arena = m2_pair(1_500);
    let opening = arena.step(0, &[], &[]);
    assert_eq!(opening.substeps, 0);
    arena.step(1_000, &[], &[]);
    let out = arena.step(2_000, &[], &[]);
    assert_eq!(out.ended.map(|end| end.t_ms), Some(1_500));
    assert_eq!(out.substeps, 2);
    assert_eq!(arena.substep_hashes.len(), 2);
}

/// **16b cut M.2** (16c `B-51`): the opening pulse drains nothing. A unit
/// whose stagger is nought has its first pass at `t = 0`, and it runs in the
/// next pulse's first substep exactly as before the drain existed — the
/// opening pulse runs no substep, so the drain's `ran > 0` guard leaves it
/// alone.
///
/// **16b cut U.1**: the stagger is `stagger_ms(seed, id, pass_ms)`, so the
/// fixture's ids are re-picked for it — `#2355`'s is nought at seed 2,350 and
/// `#9`'s is 189, inside the first substep — and both are asserted before
/// the property is.
#[test]
fn m2_the_opening_pulse_drains_nothing_and_a_pass_at_zero_runs_next_pulse() {
    use crate::arena::test_support::scout;
    const EARLY: ObjId = ObjId(2_355);
    let mut arena = crate::arena::test_support::arena_at_epoch(
        vec![
            scout(EARLY, PartyId::RED, pose(500, 500)),
            scout(ObjId(9), PartyId::BLUE, pose(4_500, 500)),
        ],
        1,
    );
    assert_eq!(arena.sensors_cfg.pass_ms, 1_000, "the default cadence");
    let stagger = |id: ObjId| crate::arena::rng::stagger_ms(arena.seed, id, 1_000);
    assert_eq!(stagger(EARLY), 0, "the precondition: a pass due at 0");
    assert!(
        (1..=arena.substep_len_ms()).contains(&stagger(ObjId(9))),
        "the precondition: a pass inside the first substep"
    );
    let opening = arena.step(0, &[], &[]);
    assert_eq!(opening.substeps, 0);
    assert_eq!(
        opening.perception.passes, 0,
        "no pass ran on the opening pulse"
    );
    assert!(
        arena.due.iter().any(|event| event.t_ms == 0
            && matches!(event.kind, DueKind::SensorPass { unit, .. } if unit == EARLY)),
        "the pass at zero left the heap"
    );
    let next = arena.step(250, &[], &[]);
    assert_eq!(next.substeps, 1);
    assert_eq!(
        next.perception.passes, 2,
        "the pass at 0 and the pass at 189 both ran in the first substep"
    );
}

/// **16b cut M.2** (16c `T-83`): the held loss is said at its own deadline,
/// by a due event, and a re-detection inside the window cancels it — the
/// `LossSettles` it left behind then fires into nothing.
#[test]
fn m2_a_held_loss_is_said_by_its_own_due_event_and_a_cancel_leaves_a_no_op() {
    const B: ObjId = ObjId(9);
    let mut arena = m2_pair(60_000);
    arena.step(0, &[], &[]);
    arena.step(1_000, &[], &[]);
    let contact = crate::arena::rng::draw_contact_id(arena.seed, A, B);
    assert!(arena.contacts.resolve(A, contact).is_some());
    let mut in_flight = None;
    // A `lost` staged at 1,250: held until 2,250 and scheduled there.
    arena.stage_pilot_notice(
        1_250,
        &EventKind::Lost {
            viewer: A,
            contact,
            target: B,
        },
        &mut in_flight,
        None,
    );
    assert_eq!(
        arena.held_losses.get(&(A, contact)).map(|(at, _)| *at),
        Some(2_250)
    );
    assert!(
        arena.due.iter().any(|event| event.t_ms == 2_250
            && event.kind == DueKind::LossSettles { viewer: A, contact }),
        "the deadline is on the heap"
    );
    let _ = arena.pending_notices.remove(&A);
    // Said at its deadline, and not before.
    arena.settle_held_loss(A, contact, 2_000);
    assert!(
        arena.held_losses.contains_key(&(A, contact)),
        "a stale settle"
    );
    arena.settle_held_loss(A, contact, 2_250);
    assert!(!arena.held_losses.contains_key(&(A, contact)));
    assert!(
        arena
            .pending_notices
            .get(&A)
            .is_some_and(|told| told.iter().any(|notice| matches!(
                notice,
                FrameNotice::Contact {
                    kind: NoticeKind::Lost,
                    ..
                }
            ))),
        "the loss was not said"
    );
}

/// **16b cut M.2** (16c `B-74`, and the review's guard on it): a cancelled
/// loss whose row came back under another name is said as an identification
/// **only** for a mesh insert that arrived told apart, and then once.
///
/// Three rows a held `X1` can come back as, each run through the staging
/// scan's two calls exactly as one substep would make them — the `detected`
/// and then the `classified` a share journals:
///
/// * a mesh insert, already `Classified` (`numbered` false): the cancel says
///   `identified` with `was: X1`, and the share's own `classified` carries no
///   `was` and renders nothing — one line;
/// * this platform's own pass re-opened the row (`numbered` true) and a share
///   classified it later in the same substep, so the scan reads it
///   `Classified`: the cancel stays silent and the share's `classified` says
///   the rename — one line, not the same sentence twice;
/// * the row is gone again by the scan (a stale mesh copy the viewer's own
///   pass expired): nothing at all, where the unguarded arm said
///   `identified: X0, unknown`.
#[test]
fn m2_a_renamed_cancel_is_said_once_and_only_for_a_mesh_insert_told_apart() {
    use crate::arena::state::Designator;
    const B: ObjId = ObjId(9);
    let mut arena = m2_pair(60_000);
    arena.step(0, &[], &[]);
    arena.step(1_000, &[], &[]);
    let contact = crate::arena::rng::draw_contact_id(arena.seed, A, B);
    assert!(arena.contacts.resolve(A, contact).is_some(), "A holds B");
    let was = Designator::Provisional(
        arena
            .designators
            .provisional(A, B)
            .unwrap_or_else(|| panic!("the fixture needs B numbered")),
    );
    let detected = EventKind::Detected {
        viewer: A,
        contact,
        target: B,
    };
    let classified = EventKind::Classified {
        viewer: A,
        contact,
        target: B,
        team: PartyId::BLUE,
    };
    // The identifications staged for `A` by one `detected` then one
    // `classified`, with the held loss and the row set up by `shape`.
    let mut run = |shape: &dyn Fn(&mut Arena)| -> Vec<FrameNotice> {
        shape(&mut arena);
        arena.held_losses.insert((A, contact), (5_000, was));
        let _ = arena.pending_notices.remove(&A);
        let mut in_flight = None;
        let _ = arena.stage_pilot_notice(1_100, &detected, &mut in_flight, None);
        if arena.contacts.resolve(A, contact).is_some() {
            let _ = arena.stage_pilot_notice(1_100, &classified, &mut in_flight, None);
        }
        assert!(
            !arena.held_losses.contains_key(&(A, contact)),
            "the `detected` did not cancel the held loss"
        );
        arena.pending_notices.remove(&A).unwrap_or_default()
    };
    let said = |notices: &[FrameNotice]| -> Vec<FrameNotice> {
        notices
            .iter()
            .filter(|notice| matches!(notice, FrameNotice::Contact { was: Some(_), .. }))
            .cloned()
            .collect()
    };

    // A mesh insert, told apart.
    let mesh = run(&|arena: &mut Arena| {
        if let Some(held) = arena.contacts.get_mut(A, contact) {
            held.numbered = false;
            held.class = Classification::Classified { party: PartyId::BLUE };
        }
    });
    let mesh_said = said(&mesh);
    assert_eq!(mesh_said.len(), 1, "one identification: {mesh:?}");
    assert!(
        matches!(
            mesh_said.first(),
            Some(FrameNotice::Contact {
                kind: NoticeKind::Classified,
                was: Some(old),
                ..
            }) if *old == was
        ),
        "the mesh insert said the rename from {was:?}: {mesh:?}"
    );

    // This platform's own row, classified by a share in the same substep.
    let own = run(&|arena: &mut Arena| {
        if let Some(held) = arena.contacts.get_mut(A, contact) {
            held.numbered = true;
            held.class = Classification::Classified { party: PartyId::BLUE };
        }
    });
    assert_eq!(
        said(&own).len(),
        1,
        "the rename was said twice (or not at all): {own:?}"
    );

    // The row already gone again.
    let gone = run(&|arena: &mut Arena| {
        let _ = arena.contacts.remove(A, contact);
    });
    assert!(
        gone.is_empty(),
        "a row that is gone was announced: {gone:?}"
    );
}

// -----------------------------------------------------------------------
// 16b cut M.3 — the lazy phase-4 hash (16c `T-23`)
// -----------------------------------------------------------------------

/// **16b cut M.3** (16c `T-23`): a pulse with nothing queued and no reflex
/// unit takes no hash in phase 4 — counted, because the slot reads `Outside`
/// after any pulse whatever phase 4 did inside it: the pulse takes exactly
/// one `state_hash` per substep (the substep's record) and phase 5's one,
/// and an eager phase 4 would be one more. Outside a run `event_hash` is a
/// fresh `state_hash` that stores nothing. A phase that resolves two
/// actions takes **one** hash between them — one more than the quiet pulse,
/// not two — and both lines carry it (the W.1 property, restated), taken at
/// its first line.
#[test]
fn m3_phase_four_takes_its_hash_at_its_first_line_and_a_quiet_pulse_takes_none() {
    const B: ObjId = ObjId(9);
    let mut arena = m2_pair(600_000);
    let mut journal = Log::default();
    journal.absorb(&arena.step(0, &[], &[]));
    let mut prior = Tempo::Quiet;
    for wall in [1_000, 2_000, 3_000] {
        let out = arena.step(wall, &[], &[]);
        prior = out.tempo;
        journal.absorb(&out);
    }
    assert!(
        arena
            .units
            .iter()
            .all(|unit| unit.controller == crate::arena::state::Controller::Pilot),
        "no machine: nothing but phase 4 could open the slot"
    );
    let taken = || crate::arena::hash::STATE_HASHES_TAKEN.with(std::cell::Cell::get);
    // A quiet pulse, counted: the substeps' records and phase 5's, and none
    // for phase 4.
    let before = taken();
    let quiet = arena.step(4_000, &[], &[]);
    let quiet_taken = taken() - before;
    journal.absorb(&quiet);
    assert!(quiet.substeps > 0, "the quiet pulse ran its substeps");
    // **16c T-104 (the walks)**: phase 5 hashes only for a line it
    // journals — its tempo line, when the tempo moved on this pulse.
    let quiet_tail = u64::from(quiet.tempo != prior);
    assert_eq!(
        quiet_taken,
        u64::from(quiet.substeps) + quiet_tail,
        "one hash per substep and phase 5's only for its line; phase 4 took none"
    );
    assert_eq!(arena.action_hash, ActionHash::Outside);
    let before = taken();
    assert_eq!(arena.event_hash(), state_hash(&arena));
    assert_eq!(taken() - before, 2, "outside a run, a fresh hash each time");
    assert_eq!(
        arena.action_hash,
        ActionHash::Outside,
        "a hash taken outside a run is not stored"
    );
    // Two discrete actions in one pulse: A's `STARTUP` and its `LOCK`.
    let contact = crate::arena::rng::draw_contact_id(arena.seed, A, B);
    let order = |kind: IntentKind, seq: u64, arena: &Arena| {
        let channel = kind.channel_required();
        Intent {
            unit: A,
            channel,
            epoch: arena.epochs.current(A, channel),
            kind,
            seq,
        }
    };
    let intents = vec![
        order(IntentKind::Startup, 1, &arena),
        order(IntentKind::Lock { contact }, 2, &arena),
    ];
    let before = taken();
    let busy = arena.step(5_000, &[], &intents);
    let busy_taken = taken() - before;
    journal.absorb(&busy);
    let busy_tail = u64::from(busy.tempo != quiet.tempo);
    assert_eq!(
        busy_taken,
        u64::from(busy.substeps) + 1 + busy_tail,
        "the two actions shared one phase-4 hash"
    );
    assert_eq!(
        arena.action_hash,
        ActionHash::Outside,
        "closed after the phase"
    );
    let log = journal.events(&mut arena);
    let phase_four: Vec<&LedgerEvent> = log
        .iter()
        .filter(|event| matches!(event.kind.as_str(), "startup" | "locked"))
        .collect();
    assert_eq!(phase_four.len(), 2, "both actions resolved: {phase_four:?}");
    assert!(
        phase_four
            .iter()
            .all(|event| event.hash == phase_four[0].hash && event.t_ms == 5_000),
        "one phase, one moment, one hash: {phase_four:?}"
    );
}

/// **16b cut Y.1**: phase 2's `ordered` lines take **one** hash between them
/// — M.3's rule (`T-23`) kept for the phase that journals now. An order to
/// two units in one pulse takes exactly one `state_hash` more than a quiet
/// pulse (the substeps' records and phase 5's), both lines carry it, both
/// sit at the pulse's opening arena millisecond, and the slot is closed
/// again after the phase.
///
/// The two scouts are handed to the machine by the controller byte alone,
/// with no think scheduled, so nothing inside the substeps can open a
/// slot of its own and the count is phase 2's.
///
/// 17b cut M4.5a: the machine is a mind, by the byte alone (no
/// `set_controller`, whose think would take a hash of its own inside the
/// substeps): the order's Mind arm lays each row with no think due, and a
/// row with no due think folds bytes but opens no slot. The release reads
/// the mind row's held order.
#[test]
fn y1_phase_two_takes_one_hash_for_every_ordered_line_of_a_pulse() {
    const B: ObjId = ObjId(9);
    let mut arena = m2_pair(600_000);
    let mut journal = Log::default();
    journal.absorb(&arena.step(0, &[], &[]));
    let mut prior = Tempo::Quiet;
    for wall in [1_000, 2_000, 3_000] {
        let out = arena.step(wall, &[], &[]);
        prior = out.tempo;
        journal.absorb(&out);
    }
    for unit in [A, B] {
        if let Some(state) = arena.unit_mut(unit) {
            state.controller = crate::arena::state::Controller::Mind;
        }
    }
    let taken = || crate::arena::hash::STATE_HASHES_TAKEN.with(std::cell::Cell::get);
    let before = taken();
    let quiet = arena.step(4_000, &[], &[]);
    let quiet_taken = taken() - before;
    journal.absorb(&quiet);
    // **16c T-104 (the walks)**: phase 5 hashes only when it journals a
    // line — here its tempo line, when the tempo moved on this pulse.
    let quiet_tail = u64::from(quiet.tempo != prior);
    assert_eq!(
        quiet_taken,
        u64::from(quiet.substeps) + quiet_tail,
        "the baseline: one hash per substep, and phase 5's only for its line"
    );
    let told = crate::deliberation::order::Order {
        squad: crate::deliberation::squad::SquadId::parse("red-1").expect("a squad id"),
        objective: crate::deliberation::order::Objective::Hold,
        target: None,
        horizon_ms: 600_000,
        constraints: crate::deliberation::order::Constraints::default(),
        ttl_ms: 600_000,
        issued_ms: 1,
        issuer: Some(A),
        // 17b cut L.1: the pilot's character, never read by the arena.
        by: crate::deliberation::order::CharacterRef::Character {
            id: mush_core::ObjId(9_001),
        },
        spot: None,
    };
    let intents: Vec<Intent> = [A, B]
        .into_iter()
        .zip(1_u64..)
        .map(|(unit, seq)| Intent {
            unit,
            channel: Channel::Helm,
            epoch: arena.epochs.current(unit, Channel::Helm),
            kind: IntentKind::Order(Box::new(told.clone())),
            seq,
        })
        .collect();
    let before = taken();
    let busy = arena.step(5_000, &[], &intents);
    let busy_taken = taken() - before;
    journal.absorb(&busy);
    let busy_tail = u64::from(busy.tempo != quiet.tempo);
    assert_eq!(
        busy_taken,
        u64::from(busy.substeps) + 1 + busy_tail,
        "the two `ordered` lines shared phase 2's one hash"
    );
    assert_eq!(
        arena.action_hash,
        ActionHash::Outside,
        "closed after the pulse"
    );
    let log = journal.events(&mut arena);
    let ordered: Vec<&LedgerEvent> = log
        .iter()
        .filter(|event| event.kind.as_str() == "ordered")
        .collect();
    assert_eq!(ordered.len(), 2, "one line per unit: {ordered:?}");
    assert!(
        ordered
            .iter()
            .all(|event| event.hash == ordered[0].hash && event.t_ms == 4_000),
        "one phase, one moment, one hash: {ordered:?}"
    );

    // **16b cut Z.2** (16c Q44): a release takes the same slot. One pulse
    // with a release for A and a fresh order for B journals one
    // `order_released` and one `ordered` line under ONE hash — one more
    // than a quiet pulse — at the pulse's opening millisecond, and counts
    // one `orders_released`.
    let squad = told.squad;
    let intents = vec![
        Intent {
            unit: A,
            channel: Channel::Helm,
            epoch: arena.epochs.current(A, Channel::Helm),
            kind: IntentKind::ReleaseOrder {
                squad,
                issuer: Some(A),
            },
            seq: 3,
        },
        Intent {
            unit: B,
            channel: Channel::Helm,
            epoch: arena.epochs.current(B, Channel::Helm),
            kind: IntentKind::Order(Box::new(told.clone())),
            seq: 4,
        },
    ];
    let before = taken();
    let mixed = arena.step(6_000, &[], &intents);
    let mixed_taken = taken() - before;
    journal.absorb(&mixed);
    let mixed_tail = u64::from(mixed.tempo != busy.tempo);
    assert_eq!(
        mixed_taken,
        u64::from(mixed.substeps) + 1 + mixed_tail,
        "the release and the order shared phase 2's one hash"
    );
    assert_eq!(mixed.motor.orders_released, 1, "one machine dropped");
    assert!(
        arena
            .minds
            .get(&A)
            .and_then(|mind| mind.order.as_ref())
            .is_none(),
        "the release landed"
    );
    let log = journal.events(&mut arena);
    let phase_two: Vec<&LedgerEvent> = log
        .iter()
        .filter(|event| {
            event.t_ms == 5_000 && matches!(event.kind.as_str(), "ordered" | "order_released")
        })
        .collect();
    assert_eq!(
        phase_two
            .iter()
            .map(|event| event.kind.as_str())
            .collect::<Vec<_>>(),
        vec!["order_released", "ordered"],
        "in the pilot's order: {phase_two:?}"
    );
    assert!(
        phase_two
            .iter()
            .all(|event| event.hash == phase_two[0].hash),
        "one phase, one moment, one hash: {phase_two:?}"
    );
}

// -----------------------------------------------------------------------
// 16b cut U.2 — the age compared at every resolution a surface derives
// (16c `T-30`, Q27, Q28)
// -----------------------------------------------------------------------

/// One contact row `age_ms` old, everything else fixed.
fn u2_row(age_ms: i64) -> FrameContact {
    FrameContact {
        id: ContactId(0x2350),
        designator: crate::arena::state::Designator::Provisional(1),
        alias: None,
        class: Classification::Unknown,
        moving: false,
        bearing_mdeg: 90_000,
        range_cm: 43_400,
        age_ms,
        cell: Some((12, 7)),
        condition: None,
        aspect: crate::arena::perception::Aspect::Front,
        facing: crate::arena::perception::ContactFacing::Front,
        reads_dead: None,
        fatal: None,
        unit_type: None,
    }
}

/// **16b cut U.2**: an unlocked row is the same row across a sub-second
/// move of its age, and a different one the moment the whole second moves
/// or the stale clause flips — at `pass_ms` 1,000 that is `1,000` against
/// `1,001`, inside one whole second.
#[test]
fn u2_an_unlocked_row_moves_with_the_second_and_the_stale_clause() {
    let same = |a: i64, b: i64| u2_row(a).same_as(&u2_row(b), 1_000, false);
    assert!(same(400, 900), "inside the first second, before the clause");
    assert!(!same(900, 1_000), "the whole second moved");
    assert!(
        !same(1_000, 1_001),
        "the stale clause flips past one pass: `, plotted 1 s ago`"
    );
    assert!(same(1_001, 1_999), "one second, stale throughout");
    assert!(!same(1_999, 2_000), "and the next second");
    // A shorter pass moves the clause and nothing else.
    assert!(!u2_row(400).same_as(&u2_row(600), 500, false));
    assert!(u2_row(600).same_as(&u2_row(900), 500, false));
}

/// **16b cut U.2**: the locked row also moves with the lock's range step,
/// which crosses from 10 m to 20 m at 648 ms — below any second — and the
/// same two ages on an unlocked row are one row.
#[test]
fn u2_a_locked_row_moves_with_the_locks_range_step() {
    use crate::arena::perception::lock_range_step_m;
    assert_eq!(lock_range_step_m(600), 10);
    assert_eq!(lock_range_step_m(700), 20);
    assert!(
        !u2_row(600).same_as(&u2_row(700), 1_000, true),
        "a locked pilot would read RANGE at a different step"
    );
    assert!(
        u2_row(600).same_as(&u2_row(700), 1_000, false),
        "an unlocked row prints no step"
    );
    assert!(
        u2_row(700).same_as(&u2_row(900), 1_000, true),
        "20 m either side"
    );
}

/// **16b cut U.2**: a reading's own age follows the same rule, and a row
/// that gained or lost its reading is a different row.
#[test]
fn u2_a_readings_age_is_compared_at_the_same_resolutions() {
    let read = |age_ms: i64| FrameContact {
        condition: Some(FrameCondition {
            table: 0,
            bands: crate::arena::condition::Condition::default(),
            age_ms,
        }),
        ..u2_row(200)
    };
    assert!(read(100).same_as(&read(800), 1_000, false));
    assert!(!read(800).same_as(&read(1_000), 1_000, false));
    assert!(!read(1_000).same_as(&read(1_001), 1_000, false));
    assert!(!read(100).same_as(&u2_row(200), 1_000, false));
    let (Some(young), Some(old)) = (read(100).condition, read(2_500).condition) else {
        panic!("both rows carry a reading");
    };
    assert!(!young.same_as(&old, 1_000));
    assert!(young.same_as(&young, 1_000));
}

/// **16b cut U.2**: `Frame::same_as` over two frames that differ only in a
/// sub-second unlocked age is `true` — nothing a pilot prints moved — and
/// over two that differ in a row's range is `false`; a row more or less is
/// always a new frame.
#[test]
fn u2_a_frame_is_sent_when_a_printed_figure_moves_and_not_per_millisecond() {
    let mut arena = fresh();
    let out = arena.step(0, &[], &[]);
    let base = out
        .frames
        .first()
        .map(|(_, frame)| frame.clone())
        .unwrap_or_else(|| panic!("the opening frame"));
    let with = |row: FrameContact| Frame {
        contacts: vec![row],
        ..base.clone()
    };
    assert!(with(u2_row(250)).same_as(&with(u2_row(750)), 1_000));
    assert!(!with(u2_row(750)).same_as(&with(u2_row(1_000)), 1_000));
    let moved = FrameContact {
        range_cm: 44_400,
        ..u2_row(250)
    };
    assert!(!with(u2_row(250)).same_as(&with(moved), 1_000));
    assert!(!with(u2_row(250)).same_as(&base, 1_000));
    // Locked on the row: the step decides.
    let locked = |row: FrameContact| Frame {
        lock: Some(row.id),
        ..with(row)
    };
    assert!(!locked(u2_row(600)).same_as(&locked(u2_row(700)), 1_000));
    assert!(locked(u2_row(100)).same_as(&locked(u2_row(600)), 1_000));
}

// -----------------------------------------------------------------------
// 16b cut I.3 — the staff floor and the staff place (16c N10)
// -----------------------------------------------------------------------

/// **16b cut I.3**: under a floor the task's wake is a WALL interval,
/// `wall + max(period, floor)`, and no due event is consulted; without one
/// the lattice rule stands.
#[test]
fn i3_next_due_under_a_floor_is_the_wall_plus_the_wake_period() {
    const WALL: i64 = 1_000_000;
    let mut arena = at_now_with(1_000, Some(1_003));
    assert_eq!(arena.next_due_ms(WALL), WALL + 250, "the lattice rule");
    arena.clock.set_floor(Some(5_000));
    assert_eq!(
        arena.next_due_ms(WALL),
        WALL + 5_000,
        "the due event 3 ms ahead is not consulted"
    );
    // Off the lattice, it is still a wall interval: nothing pulls it back.
    let mut late = at_now_with(1_253, Some(1_500));
    late.clock.set_floor(Some(5_000));
    assert_eq!(late.next_due_ms(WALL), WALL + 5_000);
    // `max(period, floor)`: a floor under the tempo's period is the period.
    arena.clock.set_floor(Some(1_000));
    assert_eq!(arena.next_due_ms(WALL), WALL + 5_000, "Quiet is 5,000");
    arena.clock.pin_tempo_for_tests(Tempo::Engaged);
    assert_eq!(arena.next_due_ms(WALL), WALL + 1_000, "Engaged under 1,000");
    arena.clock.set_floor(None);
    assert_eq!(
        arena.next_due_ms(WALL),
        WALL + 250,
        "lifted: the lattice again"
    );
}

/// **16b cut I.3, the quantisation**: a floored pulse from an off-lattice
/// `now` ends on the lattice.
#[test]
fn i3_a_floored_pulse_from_off_the_lattice_ends_on_it() {
    let mut arena = arena();
    arena.step(0, &[], &[]);
    // Unfloored, a late wake ends off the lattice, as it always has.
    arena.step(253, &[], &[]);
    assert_eq!(arena.now_ms, 253);
    arena.clock.set_floor(Some(5_000));
    let out = arena.step(5_253, &[], &[]);
    assert_eq!(
        arena.now_ms, 5_250,
        "the overshoot past the lattice is discarded"
    );
    assert_eq!(arena.now_ms % arena.substep_len_ms(), 0);
    assert!(!out.lagged);
}

/// **16b cut I.3**: an on-lattice pulse whose wall interval is the floor
/// plus a seven-millisecond overshoot moves exactly `min(cap, floor)` —
/// 2,000 at `Engaged`, 5,000 at `Quiet` — and is not lag.
#[test]
fn i3_an_on_time_floored_wake_moves_the_clamp_or_the_floor_and_no_more() {
    for (tempo, moves) in [(Tempo::Engaged, 2_000_i64), (Tempo::Quiet, 5_000)] {
        let mut arena = arena();
        arena.clock.pin_tempo_for_tests(tempo);
        arena.clock.set_floor(Some(5_000));
        arena.step(0, &[], &[]);
        let out = arena.step(5_007, &[], &[]);
        assert_eq!(arena.now_ms, moves, "{tempo:?}");
        assert!(!out.lagged, "{tempo:?}: a slowed match is not lagging");
        let out = arena.step(10_014, &[], &[]);
        assert_eq!(arena.now_ms, 2 * moves, "{tempo:?}");
        assert!(!out.lagged);
    }
}

/// **16b cut I.3**: a wake inside a substep is a pulse of no time — no
/// substep, no arena millisecond — and the pulse after a lift is still
/// quantised, the one after that not.
#[test]
fn i3_a_wake_inside_a_substep_is_a_zero_dt_pulse_and_a_lift_is_quantised_once() {
    let mut arena = arena();
    arena.clock.set_floor(Some(1_000));
    arena.step(0, &[], &[]);
    let out = arena.step(137, &[], &[]);
    assert_eq!((arena.now_ms, out.substeps), (0, 0));
    let out = arena.step(1_140, &[], &[]);
    assert_eq!(arena.now_ms, 1_000, "1,003 of wall is 1,000 of arena");
    assert_eq!(out.substeps, 4);
    arena.clock.set_floor(None);
    arena.step(2_151, &[], &[]);
    assert_eq!(
        arena.now_ms, 2_000,
        "the interval after a lift began under the floor"
    );
    arena.step(3_154, &[], &[]);
    assert_eq!(arena.now_ms, 3_003, "and the one after it is not quantised");
}

/// Every `tempo_floor` line in `log`, as `(t_ms, period_ms, hash)`.
fn i3_floor_lines(log: &[LedgerEvent]) -> Vec<(i64, Option<u32>, u64)> {
    log.iter()
        .filter_map(|event| match event.kind {
            EventKind::TempoFloor { period_ms } => Some((event.t_ms, period_ms, event.hash)),
            _ => None,
        })
        .collect()
}

/// **16b cut I.3**: the floor is journalled once per change, at the pulse's
/// end, under phase 5's one hash — the `tempo` line of the same pulse
/// carries the same — and a repeat, or a pulse with no change, journals
/// nothing.
#[test]
fn i3_the_tempo_floor_line_is_one_a_change_under_the_phase_five_hash() {
    let mut arena = arena();
    let mut journal = Log::default();
    journal.absorb(&arena.step(0, &[], &[]));
    let fire = Intent {
        unit: A,
        channel: Channel::Fire,
        epoch: 1,
        kind: IntentKind::Fire {
            weapon: SmolStr::new("cannon"),
            slot: None,
        },
        seq: 2,
    };
    arena.clock.set_floor(Some(5_000));
    journal.absorb(&arena.step(5_000, &[], &[fire]));
    journal.absorb(&arena.step(10_000, &[], &[]));
    arena.clock.set_floor(Some(5_000));
    journal.absorb(&arena.step(15_000, &[], &[]));
    arena.clock.set_floor(None);
    journal.absorb(&arena.step(20_000, &[], &[]));
    // The fire intent put the arena in `Alert` at 5,000, whose clamp is
    // 4,000: every later 5,000 ms wake moved 4,000 of arena time, so the
    // lift's pulse ended at 17,000.
    assert_eq!(arena.now_ms, 17_000);
    let log = journal.events(&mut arena);
    let floors = i3_floor_lines(&log);
    assert_eq!(
        floors
            .iter()
            .map(|(t_ms, period, _)| (*t_ms, *period))
            .collect::<Vec<_>>(),
        vec![(5_000, Some(5_000)), (17_000, None)],
        "one line a change, the repeat none: {log:?}"
    );
    assert!(
        !log.iter().any(|event| event.kind.as_str() == "lagged"),
        "a slowed match is not lagging"
    );
    let tempo_hash = log
        .iter()
        .find(|event| matches!(event.kind, EventKind::Tempo { .. }))
        .map(|event| (event.t_ms, event.hash));
    assert_eq!(
        tempo_hash,
        floors.first().map(|(t_ms, _, hash)| (*t_ms, *hash)),
        "the tempo line and the floor line of one pulse share phase 5's hash"
    );
}

/// **16b cut I.3**: never on an ended match — neither on a stop, nor on a
/// pulse whose substep ended the match.
#[test]
fn i3_no_tempo_floor_line_is_journalled_on_an_ended_match() {
    let mut stopped = arena();
    let mut journal = Log::default();
    journal.absorb(&stopped.step(0, &[], &[]));
    stopped.stop(StopReason::Abort);
    stopped.clock.set_floor(Some(5_000));
    journal.absorb(&stopped.step(5_000, &[], &[]));
    let log = journal.events(&mut stopped);
    assert!(i3_floor_lines(&log).is_empty(), "{log:?}");

    let mut timed = Arena::new(
        crate::arena::test_support::custody(
            vec![tank(A, PartyId::RED, pose(3_500, 3_500))],
            Vec::new(),
        ),
        &ArenaConfig::default(),
        &MatchConfig {
            max_duration_ms: 3_000,
            ..MatchConfig::default()
        },
        &crate::config::SensorsConfig::default(),
    )
    .expect("a three-second match");
    let mut journal = Log::default();
    journal.absorb(&timed.step(0, &[], &[]));
    journal.absorb(&timed.step(2_000, &[], &[]));
    timed.clock.set_floor(Some(5_000));
    let out = timed.step(7_000, &[], &[]);
    journal.absorb(&out);
    assert_eq!(out.ended.map(|end| end.t_ms), Some(3_000));
    let log = journal.events(&mut timed);
    assert!(i3_floor_lines(&log).is_empty(), "{log:?}");
}

// -- the place ------------------------------------------------------------

const I3_B: ObjId = ObjId(9);

/// Two tanks at epoch 1 — `A` at cell (3, 3), `B` at (5, 5) — over a window
/// whose cell (6, 2) stands 120 cm high, (1, 6) is impassable and (7, 0) is a
/// passable cell walled in on both of its window sides.
fn i3_pair() -> Arena {
    use crate::arena::state::{ArenaTerrain, TerrainCell};
    let mut arena = arena_at_epoch(
        vec![
            tank(A, PartyId::RED, pose(3_500, 3_500)),
            tank(I3_B, PartyId::BLUE, pose(5_500, 5_500)),
        ],
        1,
    );
    let ground = crate::arena::test_support::ground();
    let wall = TerrainCell {
        move_cost: 0,
        ..ground
    };
    let mut cells = vec![ground; 64];
    let mut paint = |x: usize, y: usize, cell: TerrainCell| {
        if let Some(slot) = cells.get_mut(y * 8 + x) {
            *slot = cell;
        }
    };
    paint(
        6,
        2,
        TerrainCell {
            elev_cm: 120,
            ..ground
        },
    );
    paint(1, 6, wall);
    paint(6, 0, wall);
    paint(7, 1, wall);
    std::sync::Arc::make_mut(&mut arena.definition).terrain =
        ArenaTerrain::new(8, 8, 1_000, cells).expect("the window");
    arena
}

/// A staff place of `unit` on `(x, y)` at epoch 1, as `@arena/place` stamps
/// it.
fn i3_place(unit: ObjId, x: u16, y: u16, seq: u64) -> Intent {
    order(unit, IntentKind::Place { x, y }, seq)
}

/// Every `placed` and `place_refused` line in `log`.
fn i3_place_lines(log: &[LedgerEvent]) -> Vec<LedgerEvent> {
    log.iter()
        .filter(|event| matches!(event.kind.as_str(), "placed" | "place_refused"))
        .cloned()
        .collect()
}

/// **16b cut I.3**: an accepted place puts the pose on the cell's centre at
/// its `elev_cm`, heading kept; zeroes the speed, the desired speed and the
/// carries; clears the hold, the traffic hold, `blocked` and `grade_pct`;
/// moves the index; leaves another platform's contact on the unit as it was;
/// and journals `placed` under phase 4's one hash, which the same pulse's
/// `startup` line shares. Nothing is staged.
#[test]
fn i3_a_place_moves_the_unit_and_journals_placed_under_the_phase_four_hash() {
    let mut arena = i3_pair();
    let mut journal = Log::default();
    journal.absorb(&arena.step(0, &[], &[]));
    // B holds A as a classified contact.
    let contact = crate::arena::rng::draw_contact_id(arena.seed, I3_B, A);
    arena.contacts.insert(
        I3_B,
        contact,
        crate::arena::state::ShadowContact::sighted(
            A,
            Classification::Classified { party: PartyId::RED },
            0,
            crate::arena::state::SeenPose::default(),
        ),
    );
    let before = arena.contacts.get(I3_B, contact).cloned();
    if let Some(unit) = arena.unit_mut(A) {
        unit.pose.heading_mdeg = 45_000;
        unit.speed_cms = 300;
        unit.desired_speed_cms = 500;
        unit.carry.east = 7;
        unit.carry.turn = 3;
        unit.held = Some(crate::arena::motion::Held {
            x: 3,
            y: 4,
            cause: crate::arena::motion::BlockedCause::Occupied,
        });
        unit.occupied_hold = Some(crate::arena::state::OccupiedHold {
            cell: (3, 4),
            since_ms: 0,
        });
        unit.blocked = true;
        unit.grade_pct = 12;
    }
    let startup = order(I3_B, IntentKind::Startup, 2);
    let out = arena.step(0, &[], &[i3_place(A, 6, 2, 1), startup]);
    journal.absorb(&out);
    let unit = arena.unit(A).expect("A");
    assert_eq!(
        (unit.pose.x_cm, unit.pose.y_cm, unit.pose.z_cm),
        (6_500, 2_500, 120)
    );
    assert_eq!(unit.pose.heading_mdeg, 45_000, "the heading is kept");
    assert_eq!((unit.speed_cms, unit.desired_speed_cms), (0, 0));
    assert_eq!(unit.carry, crate::arena::state::MotionCarry::default());
    assert!(unit.held.is_none() && unit.occupied_hold.is_none() && !unit.blocked);
    assert_eq!(unit.grade_pct, 0);
    assert_eq!(arena.index.occupant(6, 2), Some(A));
    assert_eq!(arena.index.occupant(3, 3), None);
    assert_eq!(arena.index.cell_of_unit(A), Some((6, 2)));
    assert_eq!(
        arena.contacts.get(I3_B, contact).cloned(),
        before,
        "the other platform's contact is untouched: same id, same class, same plot"
    );
    assert!(told(&out).is_empty(), "nothing is said to the pilot");
    let log = journal.events(&mut arena);
    let placed = i3_place_lines(&log);
    assert_eq!(
        placed.iter().map(|event| &event.kind).collect::<Vec<_>>(),
        vec![&EventKind::Placed {
            unit: A,
            x: 6,
            y: 2,
            from_x: 3,
            from_y: 3,
        }]
    );
    let startup_hash = log
        .iter()
        .find(|event| matches!(event.kind, EventKind::Startup { unit } if unit == I3_B))
        .map(|event| event.hash);
    assert_eq!(
        placed.first().map(|event| event.hash),
        startup_hash,
        "one phase-4 hash"
    );
}

/// **16b cut I.3**: each refusal journals `place_refused` with its word and
/// moves nothing — off the window, impassable, no way out, occupied — and
/// none is said to anybody.
#[test]
fn i3_a_refused_place_journals_its_word_and_moves_nothing() {
    for (x, y, word) in [
        (8_u16, 0_u16, "impassable"),
        (1, 6, "impassable"),
        (7, 0, "impassable"),
        (5, 5, "occupied"),
    ] {
        let mut arena = i3_pair();
        let mut journal = Log::default();
        journal.absorb(&arena.step(0, &[], &[]));
        let out = arena.step(0, &[], &[i3_place(A, x, y, 1)]);
        journal.absorb(&out);
        assert!(told(&out).is_empty(), "({x}, {y}) was said");
        let unit = arena.unit(A).expect("A");
        assert_eq!((unit.pose.x_cm, unit.pose.y_cm), (3_500, 3_500));
        assert_eq!(arena.index.occupant(3, 3), Some(A));
        let log = journal.events(&mut arena);
        assert_eq!(
            i3_place_lines(&log)
                .into_iter()
                .map(|event| event.kind)
                .collect::<Vec<_>>(),
            vec![EventKind::PlaceRefused {
                unit: A,
                x,
                y,
                reason: SmolStr::new_static(word),
            }],
            "({x}, {y})"
        );
    }
}

/// **16b cut I.3**: a round in flight refuses a place at EITHER end of it —
/// the target, and the shooter, whose pose the impact traces its line from.
#[test]
fn i3_a_round_in_flight_refuses_a_place_of_its_target_and_its_shooter() {
    let mut arena = m2_pair(60_000);
    let (_, seq, _) = m2_fire(&mut arena);
    assert!(arena.shot(seq).is_some(), "the round is in flight");
    for unit in [ObjId(9), A] {
        let before = arena
            .unit(unit)
            .map(|state| (state.pose.x_cm, state.pose.y_cm));
        assert_eq!(
            arena.resolve_place(unit, 2, 6, arena.now_ms),
            Err(SmolStr::new_static("in_flight")),
            "#{}",
            unit.0
        );
        assert_eq!(
            arena
                .unit(unit)
                .map(|state| (state.pose.x_cm, state.pose.y_cm)),
            before
        );
    }
    let mut journal = Log::default();
    let log = journal.events(&mut arena);
    assert_eq!(
        i3_place_lines(&log)
            .iter()
            .map(|event| event.kind.clone())
            .collect::<Vec<_>>(),
        [ObjId(9), A]
            .into_iter()
            .map(|unit| EventKind::PlaceRefused {
                unit,
                x: 2,
                y: 6,
                reason: SmolStr::new_static("in_flight"),
            })
            .collect::<Vec<_>>()
    );
}

/// **16b cut I.3**: the three gates a place skips — a wreck is placed, a
/// linkdead unit is placed, and a place spends none of the seat's eight a
/// second (a unit at its bound is still placed, and its count is untouched).
#[test]
fn i3_a_place_skips_the_wreck_the_linkdead_and_the_rate_gates() {
    type Setup = fn(&mut crate::arena::state::UnitState);
    let max = crate::config::ArenaConfig::default()
        .intents
        .max_pending_per_unit;
    let cases: [(&str, Setup); 3] = [
        ("wreck", |unit| unit.limits.wreck = true),
        ("linkdead", |unit| unit.linkdead = true),
        ("rate", |unit| {
            unit.orders_window_s = 0;
            unit.orders_in_window = crate::config::ArenaConfig::default()
                .intents
                .max_pending_per_unit;
        }),
    ];
    for (case, setup) in cases {
        let mut arena = i3_pair();
        arena.step(0, &[], &[]);
        if let Some(unit) = arena.unit_mut(A) {
            setup(unit);
        }
        let out = arena.step(0, &[], &[i3_place(A, 2, 6, 1)]);
        assert!(told(&out).is_empty(), "{case}");
        let unit = arena.unit(A).expect("A");
        assert_eq!(
            (unit.pose.x_cm, unit.pose.y_cm),
            (2_500, 6_500),
            "{case}: the unit was not placed"
        );
        if case == "rate" {
            assert_eq!(
                unit.orders_in_window, max,
                "a place spends none of the pilot's orders"
            );
        }
    }
    // And a pilot's order after a place in the same second is counted from
    // nought, as if the place had never been.
    let mut arena = i3_pair();
    arena.step(0, &[], &[]);
    arena.step(0, &[], &[i3_place(A, 2, 6, 1)]);
    assert_eq!(arena.unit(A).map(|unit| unit.orders_in_window), Some(0));
}

/// **16b cut I.3**: a place refused at phase 2 — a stale epoch here — still
/// journals `place_refused` with the phase's word under phase 2's hash, and
/// stages nothing; nor does a phase-4 refusal.
#[test]
fn i3_a_stale_place_journals_epoch_and_no_place_is_ever_said() {
    let mut arena = i3_pair();
    let mut journal = Log::default();
    journal.absorb(&arena.step(0, &[], &[]));
    let stale = Intent {
        epoch: 0,
        ..i3_place(A, 2, 6, 1)
    };
    let wrong_channel = Intent {
        channel: Channel::Fire,
        ..i3_place(A, 2, 6, 2)
    };
    let out = arena.step(0, &[], &[stale, wrong_channel, i3_place(A, 5, 5, 3)]);
    journal.absorb(&out);
    assert!(
        refusals(&out).is_empty(),
        "a place is never a Refused notice"
    );
    assert!(told(&out).is_empty());
    let log = journal.events(&mut arena);
    let lines = i3_place_lines(&log);
    assert_eq!(
        lines
            .iter()
            .map(|event| match &event.kind {
                EventKind::PlaceRefused { reason, .. } => reason.to_string(),
                other => other.as_str().to_owned(),
            })
            .collect::<Vec<_>>(),
        vec!["epoch", "channel", "occupied"]
    );
    // The two phase-2 lines share phase 2's hash; the phase-4 line has its own.
    let hashes: Vec<u64> = lines.iter().map(|event| event.hash).collect();
    assert_eq!(hashes.first(), hashes.get(1));
    assert_eq!(arena.unit(A).map(|unit| unit.pose.x_cm), Some(3_500));
}

/// **16b cut I.3**: a place that reaches the pulse a `Stop` ends the match in
/// — one the task buffered before the `Stop` arrived — never meets phase 2
/// or phase 4, and still leaves its line: `place_refused` with `stopped`,
/// just before the stop's own `aborted` line, at the same millisecond and
/// under the same hash. The unit is not moved, and nothing is said.
#[test]
fn i3_a_place_in_the_stopping_pulse_is_refused_stopped_before_the_stop_line() {
    let mut arena = i3_pair();
    let mut journal = Log::default();
    journal.absorb(&arena.step(0, &[], &[]));
    journal.absorb(&arena.step(500, &[], &[]));
    arena.stop(StopReason::Abort);
    let out = arena.step(
        700,
        &[],
        &[
            order(A, IntentKind::Startup, 1),
            i3_place(A, 2, 6, 2),
            i3_place(I3_B, 6, 6, 3),
        ],
    );
    journal.absorb(&out);
    assert_eq!(out.ended.map(|end| end.reason), Some(EndReason::Abort));
    assert!(refusals(&out).is_empty());
    assert!(told(&out).is_empty());
    let log = journal.events(&mut arena);
    // 17b cut M3.6: the two sorties close between the places and the stop
    // (was the last three lines: the two places, then the stop).
    let tail: Vec<(String, i64, u64)> = log
        .iter()
        .skip(log.len().saturating_sub(5))
        .map(|event| {
            let word = match &event.kind {
                EventKind::PlaceRefused { unit, reason, .. } => format!("{unit}:{reason}"),
                EventKind::SortieClosed { unit, word, .. } => format!("{unit}:{word}"),
                other => other.as_str().to_owned(),
            };
            (word, event.t_ms, event.hash)
        })
        .collect();
    let at = arena.now_ms;
    let hash = tail.last().map_or(0, |line| line.2);
    assert_eq!(
        tail,
        vec![
            (format!("{A}:stopped"), at, hash),
            (format!("{I3_B}:stopped"), at, hash),
            (format!("{A}:match_end"), at, hash),
            (format!("{I3_B}:match_end"), at, hash),
            ("aborted".to_owned(), at, hash),
        ],
        "the two places, the two closes, then the stop, one moment and one hash"
    );
    assert_eq!(i3_place_lines(&log).len(), 2, "and nothing else of a place");
    assert_eq!(arena.unit(A).map(|unit| unit.pose.x_cm), Some(3_500));
    assert_eq!(arena.unit(I3_B).map(|unit| unit.pose.x_cm), Some(5_500));
}

// -----------------------------------------------------------------------
// 16b cut O.1 — the Hold clock, the fresh read and the three tie rules
// -----------------------------------------------------------------------

/// The blue holder the O.1 tie tests stand on the spot: `m2_pair`'s scout at
/// (4, 0), and (with `glass`) the one-section target a single round of the
/// cannon destroys, facing west down the line of fire — the M.2 kill test's.
fn o1_holder(glass: bool) -> crate::arena::state::CustodyUnit {
    use crate::arena::resolution::{BandProfile, Consequence, SectionProfile, UnitProfile};
    use crate::arena::test_support::scout;
    let mut blue = scout(ObjId(9), PartyId::BLUE, pose(4_500, 500));
    if glass {
        blue.pose.heading_mdeg = 270_000;
        blue.profile = UnitProfile {
            bands: vec![BandProfile {
                band: SmolStr::new("front"),
                width_cm: 350,
                height_cm: 250,
                sections: vec![(SmolStr::new("hull"), 100)],
            }],
            sections: vec![SectionProfile {
                id: SmolStr::new("hull"),
                facets: vec![SmolStr::new("front")],
                consequences: vec![Consequence::DestroyUnit],
                struck_through: None,
            }],
        };
    }
    blue
}

/// **A Hold field**: `m2_pair`'s red scout `A` at (0, 0), `blue` on the key
/// spot at (4, 0) — one cell round it, `hold_ms` long — and a second blue
/// tank far off it at (7, 7), so a wrecked holder does not eliminate blue.
fn o1_hold_field(
    hold_ms: u32,
    blue: crate::arena::state::CustodyUnit,
    max_duration_ms: u32,
) -> Arena {
    use crate::arena::state::{ArenaMode, CustodyOut, HoldRule, KeySpot, MatchMode};
    use crate::arena::test_support::{custody, definition, scout};
    let mut ground = definition();
    ground.key_spots = vec![KeySpot {
        name: SmolStr::new("The Line"),
        x: 4,
        y: 0,
    }];
    ground.modes = vec![ArenaMode::Hold(HoldRule {
        spot: 0,
        radius_cells: 1,
        hold_ms,
    })];
    Arena::new(
        CustodyOut {
            definition: ground,
            mode: MatchMode::Hold,
            ..custody(
                vec![
                    scout(A, PartyId::RED, pose(500, 500)),
                    blue,
                    tank(ObjId(20), PartyId::BLUE, pose(7_500, 7_500)),
                ],
                Vec::new(),
            )
        },
        &ArenaConfig::default(),
        &MatchConfig {
            max_duration_ms,
            ..MatchConfig::default()
        },
        &crate::config::SensorsConfig::default(),
    )
    .expect("the hold field")
}

/// The `HoldSettles` events on the heap, as `(t_ms, team, since_ms)`.
fn o1_holds(arena: &Arena) -> Vec<(i64, PartyId, i64)> {
    arena
        .due
        .iter()
        .filter_map(|event| match event.kind {
            DueKind::HoldSettles { team, since_ms } => Some((event.t_ms, team, since_ms)),
            _ => None,
        })
        .collect()
}

/// **16b cut O.1**: the clock opens at the first substep boundary the sole
/// occupant is seen at, and the win is settled at `since + hold_ms` —
/// its own millisecond, inside a substep — with no line after it, and
/// `match_ended { hold }` the last line. A skirmish of the same field
/// schedules no hold at all. (Nothing else is due in that substep here; the
/// horizon's shrink over a round due after the win is
/// `o1_a_round_due_after_the_holds_millisecond_in_its_substep_is_never_applied`.)
#[test]
fn o1_a_hold_is_won_at_its_own_millisecond_and_nothing_after_it_lands() {
    // Opened at 250 (the first boundary); due at 250 + 10,100 = 10,350,
    // inside the substep 10,250..10,500.
    let mut arena = o1_hold_field(10_100, o1_holder(false), 60_000);
    let mut journal = Log::default();
    journal.absorb(&arena.step(0, &[], &[]));
    journal.absorb(&arena.step(1_000, &[], &[]));
    assert_eq!(
        arena.hold.and_then(|hold| hold.held_by),
        Some((PartyId::BLUE, 250))
    );
    assert_eq!(o1_holds(&arena), vec![(10_350, PartyId::BLUE, 250)]);
    let mut ended = None;
    for wall in (2_000..=11_000).step_by(1_000) {
        let out = arena.step(wall, &[], &[]);
        journal.absorb(&out);
        if out.ended.is_some() {
            ended = out.ended;
            break;
        }
    }
    assert_eq!(
        ended,
        Some(MatchEnd {
            reason: EndReason::Hold { winner: PartyId::BLUE },
            t_ms: 10_350
        })
    );
    assert_eq!(arena.now_ms, 10_500, "the substep the win fell in");
    let log = journal.events(&mut arena);
    assert_eq!(
        log.last().map(|event| (event.t_ms, event.kind.clone())),
        Some((
            10_350,
            EventKind::MatchEnded {
                reason: EndReason::Hold { winner: PartyId::BLUE }
            }
        ))
    );
    assert!(
        log.iter().all(|event| event.t_ms <= 10_350),
        "a line after the hold's millisecond was journalled: {log:?}"
    );
    assert_eq!(
        log.iter()
            .filter(|event| event.kind.as_str() == "match_ended")
            .count(),
        1
    );
    // The same field at a skirmish schedules no hold.
    let mut skirmish = m2_pair_with(60_000, o1_holder(false));
    skirmish.step(0, &[], &[]);
    skirmish.step(1_000, &[], &[]);
    assert!(skirmish.hold.is_none());
    assert!(o1_holds(&skirmish).is_empty());
}

/// **16b cut O.1**: a clock dropped by a contest leaves its event to fire
/// into nothing at its own millisecond; the side alone again re-opens with a
/// later `since` and a new event, and wins at that one.
#[test]
fn o1_a_contest_drops_the_clock_and_its_stale_event_fires_into_nothing() {
    let mut arena = o1_hold_field(10_000, o1_holder(false), 60_000);
    arena.step(0, &[], &[]);
    arena.step(1_000, &[], &[]);
    assert_eq!(
        arena.hold.and_then(|hold| hold.held_by),
        Some((PartyId::BLUE, 250))
    );
    // Red drives in (placed, between two pulses): contested at 1,250.
    if let Some(red) = arena.unit_mut(A) {
        red.pose = pose(3_500, 500);
    }
    arena.step(2_000, &[], &[]);
    assert_eq!(arena.hold.and_then(|hold| hold.held_by), None);
    // Red leaves: blue alone again at 2,250.
    if let Some(red) = arena.unit_mut(A) {
        red.pose = pose(500, 500);
    }
    arena.step(3_000, &[], &[]);
    assert_eq!(
        arena.hold.and_then(|hold| hold.held_by),
        Some((PartyId::BLUE, 2_250))
    );
    assert_eq!(
        o1_holds(&arena),
        vec![(10_250, PartyId::BLUE, 250), (12_250, PartyId::BLUE, 2_250)]
    );
    // The stale event pops at 10,250 and ends nothing.
    for wall in (4_000..=11_000).step_by(1_000) {
        assert_eq!(arena.step(wall, &[], &[]).ended, None, "at {wall}");
    }
    assert_eq!(o1_holds(&arena), vec![(12_250, PartyId::BLUE, 2_250)]);
    let out = arena.step(12_000, &[], &[]);
    assert_eq!(out.ended, None);
    let out = arena.step(13_000, &[], &[]);
    assert_eq!(
        out.ended,
        Some(MatchEnd {
            reason: EndReason::Hold { winner: PartyId::BLUE },
            t_ms: 12_250
        })
    );
}

/// Fire `A`'s cannon at the holder as `m2_fire` does: the journal, the
/// shot's sequence and the millisecond its round is due.
fn o1_fire(arena: &mut Arena) -> (Log, u64, i64) {
    m2_fire(arena)
}

/// **Tie rule (1)** (16b cut O.1; plan item 40): **a holder wrecked at an
/// EARLIER millisecond of the last substep voids the win.** The round lands
/// at `impact_at` and the hold is due one millisecond later in the same
/// substep: the arm's fresh read finds nobody on the spot, so it awards
/// nothing, and the substep's presence settle drops the clock. Blue still
/// has a tank, so the match runs on.
#[test]
fn o1_tie_1_a_holder_wrecked_earlier_in_the_last_substep_voids_the_win() {
    let (_, _, impact_at) = o1_fire(&mut o1_hold_field(3_600_000, o1_holder(true), 60_000));
    assert!(
        impact_at > 3_011 && impact_at < 3_250,
        "the fixture needs the round and the hold inside one substep: {impact_at}"
    );
    let hold_ms = u32::try_from(impact_at + 1 - 250).unwrap_or(u32::MAX);
    let mut arena = o1_hold_field(hold_ms, o1_holder(true), 60_000);
    let (mut journal, _, landed_at) = o1_fire(&mut arena);
    assert_eq!(landed_at, impact_at, "the probe and the run agree");
    assert_eq!(o1_holds(&arena), vec![(impact_at + 1, PartyId::BLUE, 250)]);
    let out = arena.step(4_000, &[], &[]);
    journal.absorb(&out);
    assert!(
        arena.unit(ObjId(9)).is_some_and(|unit| unit.limits.wreck),
        "the fixture needs the round to destroy the holder"
    );
    assert_eq!(out.ended, None, "a wrecked holder wins nothing");
    assert_eq!(arena.hold.and_then(|hold| hold.held_by), None);
    let log = journal.events(&mut arena);
    assert!(
        !log.iter().any(|event| event.kind.as_str() == "match_ended"),
        "{log:?}"
    );
}

/// **Tie rule (2)** (16b cut O.1; plan item 40): **a holder wrecked at the
/// hold's OWN millisecond still wins.** The hold's event was scheduled at
/// `since` — long before the shot — so it carries the lower sequence and
/// pops first; its fresh read finds the holder alive; the horizon shrinks to
/// that millisecond; the round due at it is still applied and journalled;
/// and then the match ends as `hold`, the last line.
#[test]
fn o1_tie_2_a_holder_wrecked_at_the_holds_own_millisecond_still_wins() {
    let (_, _, impact_at) = o1_fire(&mut o1_hold_field(3_600_000, o1_holder(true), 60_000));
    let hold_ms = u32::try_from(impact_at - 250).unwrap_or(u32::MAX);
    let mut arena = o1_hold_field(hold_ms, o1_holder(true), 60_000);
    let (mut journal, seq, landed_at) = o1_fire(&mut arena);
    assert_eq!(landed_at, impact_at, "the probe and the run agree");
    let hold_seq = arena
        .due
        .iter()
        .find(|event| matches!(event.kind, DueKind::HoldSettles { .. }))
        .map(|event| (event.t_ms, event.seq));
    let impact = arena
        .due
        .iter()
        .find(|event| event.kind == DueKind::Impact { shot_seq: seq })
        .map(|event| (event.t_ms, event.seq));
    assert_eq!(hold_seq.map(|(t_ms, _)| t_ms), Some(impact_at));
    assert!(
        hold_seq.map(|(_, seq)| seq) < impact.map(|(_, seq)| seq),
        "the hold pops first: {hold_seq:?} {impact:?}"
    );
    let out = arena.step(4_000, &[], &[]);
    journal.absorb(&out);
    assert_eq!(
        out.ended,
        Some(MatchEnd {
            reason: EndReason::Hold { winner: PartyId::BLUE },
            t_ms: impact_at
        })
    );
    assert!(
        arena.unit(ObjId(9)).is_some_and(|unit| unit.limits.wreck),
        "the round on the hold's millisecond was applied"
    );
    let log = journal.events(&mut arena);
    let hit = log
        .iter()
        .position(|event| event.kind.as_str() == "hit" && event.t_ms == impact_at);
    let end = log
        .iter()
        .position(|event| event.kind.as_str() == "match_ended");
    assert!(
        hit.is_some() && hit < end,
        "the hit is journalled before the end: {log:?}"
    );
    assert_eq!(
        log.last().map(|event| event.kind.clone()),
        Some(EventKind::MatchEnded {
            reason: EndReason::Hold { winner: PartyId::BLUE }
        })
    );
}

/// **The horizon** (16b cut O.1; plan items 1, 15; cut W.2's rule): a win
/// settled at its own millisecond shrinks the loop's horizon to it, exactly
/// as the deadline's arm does. The round is due ONE millisecond after the
/// hold, inside the same substep: it is never applied — the holder stands
/// whole, the `Impact` is left on the heap — nothing is journalled after the
/// hold's millisecond, and `match_ended { hold }` is the last line. Without
/// the shrink that round would wreck the holder of a match already won.
#[test]
fn o1_a_round_due_after_the_holds_millisecond_in_its_substep_is_never_applied() {
    let (_, _, impact_at) = o1_fire(&mut o1_hold_field(3_600_000, o1_holder(true), 60_000));
    assert!(
        impact_at > 3_011 && impact_at < 3_250,
        "the fixture needs the hold and the round inside one substep: {impact_at}"
    );
    let won_at = impact_at - 1;
    let hold_ms = u32::try_from(won_at - 250).unwrap_or(u32::MAX);
    let mut arena = o1_hold_field(hold_ms, o1_holder(true), 60_000);
    let (mut journal, seq, landed_at) = o1_fire(&mut arena);
    assert_eq!(landed_at, impact_at, "the probe and the run agree");
    assert_eq!(o1_holds(&arena), vec![(won_at, PartyId::BLUE, 250)]);
    let out = arena.step(4_000, &[], &[]);
    journal.absorb(&out);
    assert_eq!(
        out.ended,
        Some(MatchEnd {
            reason: EndReason::Hold { winner: PartyId::BLUE },
            t_ms: won_at
        })
    );
    assert!(
        arena.unit(ObjId(9)).is_some_and(|unit| !unit.limits.wreck),
        "the round due after the win was applied"
    );
    assert!(
            arena
                .due
                .iter()
                .any(|event| event.kind == DueKind::Impact { shot_seq: seq }
                    && event.t_ms == impact_at),
            "the round is left on the heap, never popped"
        );
    let log = journal.events(&mut arena);
    assert!(
        log.iter().all(|event| event.t_ms <= won_at),
        "a line after the hold's millisecond was journalled: {log:?}"
    );
    assert_eq!(
        log.last().map(|event| (event.t_ms, event.kind.clone())),
        Some((
            won_at,
            EventKind::MatchEnded {
                reason: EndReason::Hold { winner: PartyId::BLUE }
            }
        ))
    );
}

/// **The fresh read, the motion half** (16b cut O.1; plan item 15): **an
/// enemy that drives into the square during the last substep gives no
/// win.** The boundary before the pop read blue alone, so `held_by` still
/// names the clock when its event pops; only the fresh read at the pop —
/// over the poses this substep's motion committed — sees red inside, and it
/// awards nothing; the substep's presence settle then drops the clock. The
/// control, the same field with red kept out, wins at that millisecond.
#[test]
fn o1_an_enemy_driving_into_the_square_in_the_last_substep_voids_the_win() {
    // Opened at 250, due at 10,250: the last substep is 10,000..10,250.
    let run_to_the_last_boundary = |arena: &mut Arena| {
        for wall in (0..=10_000).step_by(1_000) {
            assert_eq!(arena.step(wall, &[], &[]).ended, None, "at {wall}");
        }
        assert_eq!(arena.now_ms, 10_000);
        assert_eq!(
            arena.hold.and_then(|hold| hold.held_by),
            Some((PartyId::BLUE, 250))
        );
        assert_eq!(o1_holds(arena), vec![(10_250, PartyId::BLUE, 250)]);
    };
    // The control: nobody comes, and blue wins at its own millisecond.
    let mut control = o1_hold_field(10_000, o1_holder(false), 60_000);
    run_to_the_last_boundary(&mut control);
    assert_eq!(
        control.step(11_000, &[], &[]).ended,
        Some(MatchEnd {
            reason: EndReason::Hold { winner: PartyId::BLUE },
            t_ms: 10_250
        })
    );
    // Red, one cell west of the square at the last boundary, driving east
    // at 5 m/s: 125 cm in the substep puts it over the square's edge.
    let mut arena = o1_hold_field(10_000, o1_holder(false), 60_000);
    run_to_the_last_boundary(&mut arena);
    if let Some(red) = arena.unit_mut(A) {
        red.pose = pose(2_950, 500);
        red.pose.heading_mdeg = 90_000;
        red.power = Power::Online;
        red.speed_cms = 500;
        red.desired_speed_cms = 500;
    }
    let cell = |arena: &Arena| {
        arena
            .unit(A)
            .and_then(|red| arena.definition.terrain.cell_of(&red.pose))
    };
    assert_eq!(cell(&arena), Some((2, 0)), "outside the square at 10,000");
    let mut journal = Log::default();
    let out = arena.step(10_250, &[], &[]);
    journal.absorb(&out);
    assert_eq!(arena.now_ms, 10_250, "the one substep the clock ran out in");
    assert!(
        cell(&arena).is_some_and(|at| arena.hold.is_some_and(|hold| hold.square_contains(at))),
        "the fixture needs red inside the square by the pop: {:?}",
        cell(&arena)
    );
    assert_eq!(out.ended, None, "a contested square wins nothing");
    assert_eq!(arena.hold.and_then(|hold| hold.held_by), None);
    assert!(o1_holds(&arena).is_empty(), "the popped event was the last");
    let log = journal.events(&mut arena);
    assert!(
        !log.iter().any(|event| event.kind.as_str() == "match_ended"),
        "{log:?}"
    );
}

/// **Tie rule (3)** (16b cut O.1; plan item 40): **a hold at the deadline's
/// own millisecond beats `Timeout`.** The deadline pops first (it was
/// scheduled at check-out), the hold second; the hold is settled before the
/// timeout arm, which then finds the match ended.
#[test]
fn o1_tie_3_a_hold_at_the_deadlines_own_millisecond_beats_the_timeout() {
    let mut arena = o1_hold_field(2_750, o1_holder(false), 3_000);
    let mut journal = Log::default();
    let mut ended = None;
    for wall in [0, 1_000, 2_000, 3_000] {
        let out = arena.step(wall, &[], &[]);
        journal.absorb(&out);
        ended = ended.or(out.ended);
    }
    assert_eq!(
        ended,
        Some(MatchEnd {
            reason: EndReason::Hold { winner: PartyId::BLUE },
            t_ms: 3_000
        })
    );
    let log = journal.events(&mut arena);
    let ends: Vec<(i64, EventKind)> = m2_ends(&log);
    assert_eq!(
        ends,
        vec![(
            3_000,
            EventKind::MatchEnded {
                reason: EndReason::Hold { winner: PartyId::BLUE }
            }
        )],
        "one end, the hold, and no timeout beside it"
    );
}

// -----------------------------------------------------------------------
// 17b cut M1a.3 — the frame grows: aspect, odds, `reads_dead`, order,
// `t_ms` (doc 17 §3, D-UM-4, D-UM-5)
// -----------------------------------------------------------------------

/// The m1a3 field: RED, the platform, in the south-west; five other stock
/// tanks, each held by RED under an id the tests choose.
mod m1a3 {
    use super::super::*;
    use crate::arena::condition::{Condition, ConditionBand, Reading};
    use crate::arena::perception::{Notice, PerceptionCounters};
    use crate::arena::state::{SeenPose, ShadowContact, PartyId, WeaponSummary};
    use crate::arena::test_support::{arena_of, cannon, line_tank, pose};

    pub(super) const RED: ObjId = ObjId(4);
    /// Classified hostile, 39 m due north — the nearest classified contact —
    /// facing north: RED sees its rear. Held under the id RED's own sensor
    /// pass would mint for it, so a pass refreshes this row.
    pub(super) const BLUE: ObjId = ObjId(9);
    /// Classified FRIEND, 40 m due east.
    pub(super) const MATE: ObjId = ObjId(11);
    /// Classified hostile, also exactly 40 m away — the range tie.
    pub(super) const TIE: ObjId = ObjId(12);
    /// Classified hostile, farthest, under the LOWEST id.
    pub(super) const FAR: ObjId = ObjId(13);
    /// Unclassified, the nearest of all.
    pub(super) const NEAR: ObjId = ObjId(14);

    pub(super) const MATE_ID: ContactId = ContactId(20);
    pub(super) const TIE_ID: ContactId = ContactId(30);
    pub(super) const FAR_ID: ContactId = ContactId(5);
    pub(super) const NEAR_ID: ContactId = ContactId(40);

    /// BLUE's row: the id `draw_contact_id` mints for the pair.
    pub(super) fn blue_id(arena: &Arena) -> ContactId {
        crate::arena::rng::draw_contact_id(arena.seed, RED, BLUE)
    }

    /// The field, with every hold written by hand at `t = 0` and RED locked
    /// on BLUE.
    pub(super) fn field() -> Arena {
        let mut arena = arena_of(vec![
            line_tank(RED, PartyId::RED, pose(1_500, 1_500)),
            line_tank(BLUE, PartyId::BLUE, pose(1_500, 5_400)),
            line_tank(MATE, PartyId::RED, pose(5_500, 1_500)),
            // (2,400, 3,200) from RED: exactly 4,000 cm, as BLUE and MATE.
            line_tank(TIE, PartyId::BLUE, pose(3_900, 4_700)),
            line_tank(FAR, PartyId::BLUE, pose(6_500, 6_500)),
            line_tank(NEAR, PartyId::BLUE, pose(3_500, 2_500)),
        ]);
        if let Some(red) = arena.unit_mut(RED) {
            red.power = Power::Online;
        }
        let classified = |team: PartyId| Classification::Classified { party: team };
        for (id, target, class) in [
            (blue_id(&arena), BLUE, classified(PartyId::BLUE)),
            (MATE_ID, MATE, classified(PartyId::RED)),
            (TIE_ID, TIE, classified(PartyId::BLUE)),
            (FAR_ID, FAR, classified(PartyId::BLUE)),
            (NEAR_ID, NEAR, Classification::Unknown),
        ] {
            let seen = SeenPose::of(arena.unit(target).unwrap_or_else(|| panic!("{target}")));
            arena
                .contacts
                .insert(RED, id, ShadowContact::sighted(target, class, 0, seen));
        }
        let lock = blue_id(&arena);
        if let Some(red) = arena.unit_mut(RED) {
            red.lock = Some(lock);
        }
        arena
    }

    /// **The far field**: RED and BLUE alone, 300 m apart on a window tall
    /// enough to hold them (8 by 36 cells), BLUE held classified and locked
    /// by its minted id, facing north so RED sees its rear.
    ///
    /// At the [`field`]'s 39 m every figure sits at the hit curve's ceiling
    /// (`p_max`), so a frame that read the live target there would print the
    /// same number as one that read the plot. At 300 m the stock cannon's
    /// figure is mid-curve, where a range, a face or a crossing speed each
    /// move it — the scene in which "the odds are the plot's" can fail.
    pub(super) fn far_field() -> Arena {
        use crate::arena::state::{ArenaDefinition, ArenaTerrain, CustodyOut, MatchMode};
        use crate::arena::test_support::{definition, ground, resolution};
        use crate::config::{ArenaConfig, MatchConfig, SensorsConfig};
        let mut arena = Arena::new(
            CustodyOut {
                definition: ArenaDefinition {
                    terrain: ArenaTerrain::uniform(8, 36, 1_000, ground())
                        .unwrap_or_else(|error| panic!("the far window: {error:?}")),
                    ..definition()
                },
                units: vec![
                    line_tank(RED, PartyId::RED, pose(1_500, 1_500)),
                    line_tank(BLUE, PartyId::BLUE, pose(1_500, 31_500)),
                ],
                pilots: Vec::new(),
                resolution: resolution(),
                seed: 2_350,
                mode: MatchMode::Skirmish,
            },
            &ArenaConfig::default(),
            &MatchConfig::default(),
            &SensorsConfig::default(),
        )
        .unwrap_or_else(|error| panic!("the far arena: {error:?}"));
        if let Some(red) = arena.unit_mut(RED) {
            red.power = Power::Online;
        }
        let id = blue_id(&arena);
        let seen = SeenPose::of(arena.unit(BLUE).unwrap_or_else(|| panic!("BLUE")));
        arena.contacts.insert(
            RED,
            id,
            ShadowContact::sighted(
                BLUE,
                Classification::Classified { party: PartyId::BLUE },
                0,
                seen,
            ),
        );
        if let Some(red) = arena.unit_mut(RED) {
            red.lock = Some(id);
        }
        arena
    }

    /// RED's frame, built now.
    pub(super) fn red(arena: &Arena) -> Frame {
        arena
            .frame_for(RED, Tempo::Engaged)
            .unwrap_or_else(|| panic!("RED's frame"))
    }

    /// One row of a frame, by id.
    pub(super) fn row(frame: &Frame, id: ContactId) -> FrameContact {
        frame
            .contacts
            .iter()
            .find(|held| held.id == id)
            .cloned()
            .unwrap_or_else(|| panic!("row {id:?}"))
    }

    /// The odds of one row, as `(weapon, p_permille)`.
    pub(super) fn odds_of(frame: &Frame, id: ContactId) -> Vec<(SmolStr, u32)> {
        frame
            .odds
            .iter()
            .filter(|odds| odds.contact == id)
            .map(|odds| (odds.weapon.clone(), odds.p_permille))
            .collect()
    }

    /// A reading taken at `t = 0` with one section destroyed.
    pub(super) fn reading_with(destroyed: usize) -> Reading {
        Reading {
            at_ms: 0,
            condition: Condition::default().with_band(destroyed, ConditionBand::Destroyed),
        }
    }

    /// RED's own sensor pass over the field, journalled nowhere.
    pub(super) fn pass(arena: &mut Arena, pass_seq: u64, t_ms: i64) {
        let mut counters = PerceptionCounters::default();
        let mut journal: Vec<Notice> = Vec::new();
        arena.sensor_pass(RED, pass_seq, t_ms, &mut counters, &mut journal);
    }

    /// The stock cannon under another name, for the weapon-order test.
    pub(super) fn gun(id: &str) -> WeaponSummary {
        WeaponSummary {
            id: SmolStr::new(id),
            ..cannon()
        }
    }
}

/// **17b cut M1a.3, the observer-rule assertion for every new field** (doc 17
/// §3, D-UM-5; doc 16 §1, §5): `aspect`, `odds`, `reads_dead` and `fatal`
/// are read off the plot, the hold and the target chassis's content — never
/// the target's live pose, heading or wreck flag. Moving and turning a held
/// tank behind the platform's back changes none of them; the platform's own
/// next pass does. The odds half runs twice: on the near field, where it
/// pins which rows carry odds, and on the far field, where the figure is
/// mid-curve and a live read would print another number. Which contacts
/// carry odds (the lock first, then the nearest classified hostiles, never
/// a friend or an unclassified row — 17b cut M1b.3's Q2), and the
/// `reads_dead` rule in all its arms, are pinned here too.
#[test]
fn m1a3_aspect_odds_and_reads_dead_are_read_off_the_plot_and_the_hold() {
    use crate::arena::perception::Aspect;
    use crate::arena::state::SeenPose;
    use m1a3::*;
    let mut arena = field();
    let blue_id = blue_id(&arena);
    let before = red(&arena);
    let blue = row(&before, blue_id);
    assert_eq!(
        blue.aspect,
        Aspect::Rear,
        "BLUE faces north and RED is due south of it"
    );
    assert_eq!(blue.reads_dead, Some(false));
    assert_eq!(blue.fatal, None, "no reading, no fatal words");
    assert_eq!(before.t_ms, arena.now_ms);
    assert_eq!(before.order, None, "a piloted hull carries no order");

    // Which rows carry odds — **17b cut M1b.3** (Q2, the lead's ruling):
    // BLUE (39 m, the lock, row one) and TIE (40 m, the nearest other
    // hostile); never MATE (also 40 m and the lower id, but red's own:
    // the rows never carry a friend's — under 17b cut M1a.3 it carried
    // odds, doc 17 §3's *any classified contact*), not FAR (the farthest)
    // and never NEAR (the nearest of all, but unclassified).
    let carrying: Vec<ContactId> = before.odds.iter().map(|odds| odds.contact).collect();
    assert_eq!(
        carrying,
        vec![blue_id, TIE_ID],
        "the lock first, then the nearest hostile"
    );
    assert!(
        odds_of(&before, MATE_ID).is_empty(),
        "never a friend's, however near"
    );
    assert!(odds_of(&before, FAR_ID).is_empty());
    assert!(
        odds_of(&before, NEAR_ID).is_empty(),
        "never for an unclassified contact"
    );
    assert_eq!(STOCK_ODDS_ROWS, 2);
    // The unclassified row is still told its aspect, off its plot.
    assert_eq!(row(&before, NEAR_ID).aspect, Aspect::Left);
    // And the locked row's odds are the lock's own percentage: one reading
    // of one plot (`aim_at` and `lock_aim` are one body).
    let blue_odds = odds_of(&before, blue_id);
    assert_eq!(blue_odds.len(), 1, "the line tank carries one gun");
    assert_eq!(
        blue_odds.first().map(|(_, p)| Some(*p)),
        before.weapons.first().map(|weapon| weapon.p_permille),
        "the row's odds are the weapon row's lock chance"
    );
    assert!(blue_odds.iter().all(|(_, p)| *p > 0), "{blue_odds:?}");

    // Drive BLUE two cells north, turn it about and make it a wreck, by
    // writing the arena with NO pass: nothing the platform holds moves.
    if let Some(target) = arena.unit_mut(BLUE) {
        target.pose.y_cm += 2_000;
        target.pose.heading_mdeg = 180_000;
        target.speed_cms = 900;
        target.limits.wreck = true;
    }
    let hidden = red(&arena);
    let still = row(&hidden, blue_id);
    assert_eq!(still.aspect, blue.aspect, "the aspect is the plot's");
    assert_eq!(still.reads_dead, Some(false), "never the live wreck flag");
    assert_eq!(still.fatal, blue.fatal);
    assert_eq!(still.range_cm, blue.range_cm);
    assert_eq!(
        odds_of(&hidden, blue_id),
        blue_odds,
        "the odds are the plot's"
    );
    assert!(hidden.same_as(&before, 1_000), "nothing to send");
    // And what the odds are resolved against is still the old plot — the
    // seen heading, the seen speed, the range to it and its cover — which is
    // what holds the figure still when it is not at the curve's ceiling.
    let held_aim = arena.aim_at(RED, blue_id);
    assert_eq!(held_aim.map(|aim| aim.seen_at.heading_mdeg), Some(0));
    assert_eq!(held_aim.map(|aim| aim.seen_at.speed_cms), Some(0));
    assert_eq!(held_aim.map(|aim| aim.range_cm), Some(blue.range_cm));

    // The platform's own pass sees the move: the plot, and with it the
    // aspect and the odds, move. (A stock sight does not report a wreck,
    // so BLUE is whole again for the pass; and the hand-written holds are
    // dropped, because the pass re-opens each under its own minted id,
    // unclassified, which leaves BLUE the one classified row.)
    if let Some(target) = arena.unit_mut(BLUE) {
        target.limits.wreck = false;
    }
    for id in [MATE_ID, TIE_ID, FAR_ID, NEAR_ID] {
        arena.contacts.remove(RED, id);
    }
    arena.index.rebuild(&arena.definition.terrain, &arena.units);
    pass(&mut arena, 2, 1_000);
    arena.now_ms = 1_000;
    let seen = red(&arena);
    let moved = row(&seen, blue_id);
    assert_eq!(moved.aspect, Aspect::Front, "turned about, seen nose-on");
    assert!(moved.range_cm > blue.range_cm, "{moved:?}");
    // The odds are re-read off the moved plot. (At 39 m and 59 m both sit
    // at the curve's ceiling, so the figure is the same number here; what
    // moved is the plot it was resolved against, which is the one `aim_at`
    // answers now. The far field below is where the figure itself moves.)
    let aim = arena
        .aim_at(RED, blue_id)
        .unwrap_or_else(|| panic!("BLUE is held"));
    assert_eq!(aim.range_cm, moved.range_cm, "the aim is the new plot's");
    assert_eq!(aim.seen_at.heading_mdeg, 180_000);
    assert_eq!(
        odds_of(&seen, blue_id),
        vec![(
            SmolStr::new_static("cannon"),
            arena.resolve_seen(RED, &aim, "cannon").p_permille
        )]
    );

    // **The odds proof where it can fail.** At 300 m the cannon's figure
    // is mid-curve, so the plot and the live tank give different numbers
    // once the tank has moved: this half is what catches a `frame_odds`
    // that resolved against the live target.
    let mut far = far_field();
    let far_id = m1a3::blue_id(&far);
    let plotted = red(&far);
    let figure = odds_of(&plotted, far_id);
    let p = figure.first().map_or(0, |(_, p)| *p);
    assert!(
        p > far.resolution.p_min && p < far.resolution.p_max,
        "mid-curve, not at a clamp: {figure:?}"
    );
    // Drive BLUE 20 m on, turn it broadside and set it crossing, with no
    // pass: a farther range, a face twice as wide and a crossing speed —
    // three live inputs that each move a figure off the ceiling.
    if let Some(target) = far.unit_mut(BLUE) {
        target.pose.y_cm += 2_000;
        target.pose.heading_mdeg = 90_000;
        target.speed_cms = 900;
    }
    let hidden = red(&far);
    assert_eq!(odds_of(&hidden, far_id), figure, "the odds are the plot's");
    assert_eq!(row(&hidden, far_id).aspect, Aspect::Rear);
    assert!(hidden.same_as(&plotted, 1_000), "nothing to send");
    // And the two live reads the frame must not make are other numbers
    // here: the shot's own physics, and the plot's arithmetic over a
    // sighting taken now.
    let aim = far
        .aim_at(RED, far_id)
        .unwrap_or_else(|| panic!("BLUE is held"));
    let (Some(from), Some(at)) = (far.unit(RED), far.unit(BLUE)) else {
        panic!("both tanks");
    };
    let live_seen = SeenPose::of(at);
    let live_range = live_seen.range_from(&from.pose);
    assert_ne!(
        far.resolve_shot(RED, BLUE, "cannon", live_range, aim.cover)
            .p_permille,
        p,
        "the live shot is another number"
    );
    let live_aim = crate::arena::resolution::LockAim {
        seen_at: live_seen,
        range_cm: live_range,
        ..aim
    };
    assert_ne!(
        far.resolve_seen(RED, &live_aim, "cannon").p_permille,
        p,
        "a sighting taken now is another number"
    );
    // The platform's own pass moves the plot, and the figure with it — to
    // exactly what the new plot resolves at. The frame's clock is the
    // state's.
    far.index.rebuild(&far.definition.terrain, &far.units);
    pass(&mut far, 2, 1_000);
    far.now_ms = 1_250;
    let refreshed = red(&far);
    assert_eq!(refreshed.t_ms, 1_250, "t_ms is the state's clock");
    let aim = far
        .aim_at(RED, far_id)
        .unwrap_or_else(|| panic!("BLUE is still held"));
    assert_eq!(aim.seen_at.heading_mdeg, 90_000, "the pass saw the turn");
    let moved_figure = odds_of(&refreshed, far_id);
    assert_ne!(moved_figure, figure, "a pass moves the odds");
    assert_eq!(
        moved_figure,
        vec![(
            SmolStr::new_static("cannon"),
            far.resolve_seen(RED, &aim, "cannon").p_permille
        )]
    );
    assert_ne!(row(&refreshed, far_id).aspect, Aspect::Rear);
    assert!(!refreshed.same_as(&plotted, 1_000), "and the frame is sent");

    // `reads_dead`, arm by arm, on a fresh field.
    let mut arena = field();
    assert_eq!(row(&red(&arena), TIE_ID).reads_dead, Some(false));
    let blue_id = m1a3::blue_id(&arena);
    // A reading whose destroyed section is not fatal (the turret, index 4
    // of the stock tank's sections in id order): false, and the fatal
    // words beside it.
    if let Some(held) = arena.contacts.get_mut(RED, TIE_ID) {
        held.seen_condition = Some(reading_with(4));
    }
    let read = row(&red(&arena), TIE_ID);
    assert_eq!(read.reads_dead, Some(false));
    assert_eq!(read.fatal, Some(vec![SmolStr::new_static("hull")]));
    // A reading with the hull destroyed (index 1): true.
    if let Some(held) = arena.contacts.get_mut(RED, TIE_ID) {
        held.seen_condition = Some(reading_with(1));
    }
    assert_eq!(row(&red(&arena), TIE_ID).reads_dead, Some(true));
    // RED's own kill, with no reading at all: true, and still no words.
    if let Some(held) = arena.contacts.get_mut(RED, FAR_ID) {
        held.known_dead = true;
    }
    let killed = row(&red(&arena), FAR_ID);
    assert_eq!(killed.reads_dead, Some(true));
    assert_eq!(killed.fatal, None);
    // Value-identical to `contact_reads_dead` wherever a reading is held
    // (doc 17 §17's divergence (4), measured honestly), and `fatal`
    // present exactly when a reading is.
    if let Some(held) = arena.contacts.get_mut(RED, MATE_ID) {
        held.seen_condition = Some(reading_with(2));
    }
    let frame = red(&arena);
    let mut compared = 0;
    for (id, held) in arena.contacts.of(RED) {
        let row = row(&frame, *id);
        assert_eq!(row.fatal.is_some(), held.seen_condition.is_some(), "{id:?}");
        if held.seen_condition.is_some() {
            assert_eq!(
                row.reads_dead,
                Some(arena.contact_reads_dead(held)),
                "{id:?}"
            );
            compared += 1;
        }
    }
    assert_eq!(compared, 2, "TIE (the hull) and MATE (a track)");

    // An unarmed platform: no odds, no `reads_dead` anywhere, the aspect
    // and the fatal words still told, and no weapons.
    if let Some(red) = arena.unit_mut(RED) {
        red.weapons.clear();
    }
    let unarmed = red(&arena);
    assert!(unarmed.odds.is_empty());
    assert!(unarmed.weapons.is_empty());
    assert!(unarmed.contacts.iter().all(|row| row.reads_dead.is_none()));
    assert_eq!(row(&unarmed, blue_id).aspect, Aspect::Rear);
    assert_eq!(
        row(&unarmed, TIE_ID).fatal,
        Some(vec![SmolStr::new_static("hull")]),
        "the fatal words ride the reading, which is the sensors'"
    );
}

/// **Doc 22 D-22-9**: `FrameContact::facing` reports all eight 45° sectors off
/// the contact's plot, while `FrameContact::aspect` stays the four-way armour
/// facet.
#[test]
fn d22_9_facing_reports_all_eight_sectors_off_the_plot_while_aspect_stays_four() {
    use crate::arena::perception::{Aspect, ContactFacing};
    use m1a3::*;
    let mut arena = far_field();
    let blue_id = blue_id(&arena);
    // In `far_field`, RED is directly south of BLUE, so the bearing from BLUE
    // to RED is 180° (south). As BLUE's plotted heading sweeps in 45° steps
    // from 180° (facing south, nose-on to RED) counter-clockwise, RED sees
    // front, front_right, right, rear_right, rear, rear_left, left, front_left.
    let expected = [
        (180_000, ContactFacing::Front, Aspect::Front),
        (135_000, ContactFacing::FrontRight, Aspect::Right),
        (90_000, ContactFacing::Right, Aspect::Right),
        (45_000, ContactFacing::RearRight, Aspect::Rear),
        (0, ContactFacing::Rear, Aspect::Rear),
        (315_000, ContactFacing::RearLeft, Aspect::Left),
        (270_000, ContactFacing::Left, Aspect::Left),
        (225_000, ContactFacing::FrontLeft, Aspect::Front),
    ];
    for (heading_mdeg, want_facing, want_aspect) in expected {
        if let Some(held) = arena.contacts.get_mut(RED, blue_id) {
            held.seen_at.heading_mdeg = heading_mdeg;
        }
        let f = red(&arena);
        let r = row(&f, blue_id);
        assert_eq!(r.facing, want_facing, "heading {heading_mdeg}");
        assert_eq!(r.aspect, want_aspect, "heading {heading_mdeg}");
    }
}

/// **17b cut M1a.3**: `Frame::same_as` compares each new field at the
/// resolution a surface prints it at — an aspect flip re-sends, a per-mille
/// move inside one percent does not, `t_ms` alone never does, an order
/// change does, and so does `reads_dead`.
#[test]
fn m1a3_same_as_compares_the_new_fields_at_their_printed_resolution() {
    use crate::arena::perception::{Aspect, FrameOrder, MindObjective};
    use m1a3::*;
    let arena = field();
    let base = red(&arena);
    assert!(base.same_as(&base.clone(), 1_000));
    // `t_ms` alone: the same frame.
    let later = Frame {
        t_ms: base.t_ms + 777,
        ..base.clone()
    };
    assert!(later.same_as(&base, 1_000), "t_ms is never compared");
    // An aspect flip on one row.
    let mut flipped = base.clone();
    let blue_id = blue_id(&arena);
    if let Some(row) = flipped.contacts.iter_mut().find(|row| row.id == blue_id) {
        row.aspect = Aspect::Left;
    }
    assert!(!flipped.same_as(&base, 1_000));
    // `reads_dead`.
    let mut dead = base.clone();
    if let Some(row) = dead.contacts.iter_mut().find(|row| row.id == blue_id) {
        row.reads_dead = Some(true);
    }
    assert!(!dead.same_as(&base, 1_000));
    // `fatal`.
    let mut words = base.clone();
    if let Some(row) = words.contacts.iter_mut().find(|row| row.id == blue_id) {
        row.fatal = Some(vec![SmolStr::new_static("hull")]);
    }
    assert!(!words.same_as(&base, 1_000));
    // The odds at the whole percent: 620..=629 all print `62%`.
    let with_permille = |p: u32| {
        let mut frame = base.clone();
        if let Some(first) = frame.odds.first_mut() {
            first.p_permille = p;
        }
        frame
    };
    assert!(with_permille(620).same_as(&with_permille(629), 1_000));
    assert!(!with_permille(629).same_as(&with_permille(630), 1_000));
    // One entry more or less is a different frame.
    let mut fewer = base.clone();
    fewer.odds.pop();
    assert!(!fewer.same_as(&base, 1_000));
    // An order, and a change of it.
    let order = |facing_mdeg: u32| FrameOrder {
        identity: None,
        squad: SmolStr::new_static("red-1"),
        objective: MindObjective::Screen { facing_mdeg },
        hold_fire: false,
        max_range_band: None,
        expires_ms: 30_000,
    };
    let ordered = Frame {
        order: Some(order(90_000)),
        ..base.clone()
    };
    assert!(!ordered.same_as(&base, 1_000));
    assert!(ordered.same_as(&ordered.clone(), 1_000));
    let turned = Frame {
        order: Some(order(180_000)),
        ..base.clone()
    };
    assert!(!turned.same_as(&ordered, 1_000));
}

/// **17b cut M1a.3**: a contact's odds lead with the weapon a bare `FIRE`
/// would fire whatever the component order, so a client reading the first
/// entry and line mode — which prints `fired_by_default`'s — quote one gun;
/// the rest follow in component order.
#[test]
fn m1a3_the_odds_array_leads_with_the_default_weapon() {
    use m1a3::*;
    let mut arena = field();
    if let Some(red) = arena.unit_mut(RED) {
        red.weapons = vec![gun("machine_gun"), gun("cannon"), gun("autocannon")];
    }
    let frame = red(&arena);
    let default = crate::arena::view::fired_by_default(&frame.weapons)
        .map(|weapon| weapon.id.clone())
        .unwrap_or_else(|| panic!("a default weapon"));
    assert_eq!(default.as_str(), "cannon");
    // 17b cut M1b.3 (Q2): the lock and the nearest hostile — MATE, a
    // friend, never carries odds.
    for id in [blue_id(&arena), TIE_ID] {
        let order: Vec<SmolStr> = odds_of(&frame, id).into_iter().map(|(w, _)| w).collect();
        assert_eq!(
            order,
            vec![
                SmolStr::new_static("cannon"),
                SmolStr::new_static("machine_gun"),
                SmolStr::new_static("autocannon"),
            ],
            "the default first, then component order"
        );
    }
    // With no `cannon` aboard, `fired_by_default` is the first carried —
    // and so is the array's first entry.
    if let Some(red) = arena.unit_mut(RED) {
        red.weapons = vec![gun("autocannon"), gun("machine_gun")];
    }
    let frame = red(&arena);
    assert_eq!(
        frame.odds.first().map(|odds| odds.weapon.clone()),
        crate::arena::view::fired_by_default(&frame.weapons).map(|weapon| weapon.id.clone())
    );
}

/// **A mind closing on a contact raises `Alert`** (17b cut M4.3; doc 16
/// §11): the tempo's `ai_approaching` reads either machine — a mind's
/// through its cached doctrine state, the state its last think WROTE —
/// so a mind hull whose row reads `Approach` raises `Tempo::Alert` on
/// the next pulse, one substep after the reflex machine's tick would
/// have; the same row dormant under a pilot raises nothing.
#[test]
fn m4_3_a_mind_in_approach_raises_alert_and_a_dormant_row_does_not() {
    use crate::arena::doctrine::MachineState;
    use crate::arena::mind::{doctrine_state_of, encode_memory};
    use crate::arena::state::Controller;
    use crate::arena::stock_mind::StockMemory;
    let approaching = |arena: &mut Arena| {
        if let Some(state) = arena.minds.get_mut(&A) {
            state.memory = encode_memory(&crate::arena::mind::Memory::from(StockMemory {
                state: MachineState::Approach,
                ..StockMemory::default()
            }));
            state.doctrine_state = doctrine_state_of(&state.memory);
        }
    };
    let mut minded = arena();
    minded.mind_runner = std::sync::Arc::new(crate::arena::mind::NoMind);
    minded.step(0, &[], &[]);
    assert_eq!(minded.clock.tempo(), Tempo::Quiet, "nothing is approaching");
    minded.set_controller(A, Controller::Mind, MachineState::Startup);
    approaching(&mut minded);
    assert!(minded.any_mind_approaching());
    let out = minded.step(1_000, &[], &[]);
    assert_eq!(out.tempo, Tempo::Alert, "a mind in Approach raises Alert");

    let mut dormant = arena();
    dormant.mind_runner = std::sync::Arc::new(crate::arena::mind::NoMind);
    dormant.step(0, &[], &[]);
    dormant.set_controller(A, Controller::Mind, MachineState::Startup);
    approaching(&mut dormant);
    dormant.set_controller(A, Controller::Pilot, MachineState::Startup);
    assert!(
        dormant
            .minds
            .get(&A)
            .is_some_and(|state| state.doctrine_state == MachineState::Approach),
        "the row is dormant, its state still Approach"
    );
    assert!(
        !dormant.any_mind_approaching(),
        "a dormant row is never read"
    );
    let out = dormant.step(1_000, &[], &[]);
    assert_eq!(out.tempo, Tempo::Quiet);
}

/// The fixture tank, seated, sealed with Standard Doctrine on `profiles`
/// (17b cut M5.4) under the fixture terms `terms` turns them into.
fn m5_sealed(
    unit: crate::arena::state::CustodyUnit,
    profiles: &[&str],
    terms: crate::match_::SealTerms,
) -> crate::arena::state::CustodyUnit {
    let reflex = crate::config::ReflexConfig::default();
    let cards = profiles
        .iter()
        .map(|name| {
            let card = crate::arena::card::preset(name, &reflex)
                .unwrap_or_else(|| panic!("{name} is a preset"));
            (SmolStr::new(*name), card)
        })
        .collect();
    let mut unit = unit;
    unit.reseal(&terms.with_profiles(cards))
        .unwrap_or_else(|error| panic!("the fixture seals: {error:?}"));
    unit
}

/// **17b cut M5.4** (doc 17 §12; the M5 design note's M5.4 item 2): a
/// switch alone changes the frame. A seated, cold, stationary tank sealed
/// on Sniper and Line is sent its frame once and nothing more; then its
/// pilot's `MIND PROFILE Line` (`IntentKind::Profile { index: 1 }`) lands
/// in a pulse that advances no time and moves nothing, and that pulse
/// sends one frame whose `mind` reads the new profile and the ABSOLUTE
/// moment of the next switch — the frame equal to the one before in every
/// field but `mind`, so without [`Frame::same_as`]'s comparison of it the
/// pilot would have been sent nothing. A quiet pulse after it sends
/// nothing again: `switch_at_ms` does not move with the clock.
#[test]
fn m5_a_switch_alone_changes_the_frame() {
    use crate::arena::core::CoreClass;
    let unit = m5_sealed(
        tank(A, PartyId::RED, pose(3_500, 3_500)),
        &["Sniper", "Line"],
        crate::match_::SealTerms::fixture(),
    );
    let mut arena = arena_at_epoch(vec![unit], 1);
    let opening = arena.step(1_000_000, &[], &[]);
    let Some((_, before)) = opening.frames.first().cloned() else {
        panic!("the pilot's first frame");
    };
    let Some(mind) = before.mind.as_ref() else {
        panic!("a sealed hull's frame carries its mind");
    };
    assert!(mind.doctrine);
    assert_eq!(
        mind.profiles.names().collect::<Vec<_>>(),
        ["Sniper", "Line"]
    );
    assert_eq!((mind.active, mind.switch_at_ms, mind.off), (0, None, false));
    assert_eq!((mind.class, mind.core_class), (CoreClass::I, CoreClass::I));
    assert!(arena.step(1_000_000, &[], &[]).frames.is_empty(), "quiet");

    let switch = Intent {
        unit: A,
        channel: Channel::Helm,
        epoch: 1,
        kind: IntentKind::Profile { index: 1 },
        seq: 1,
    };
    let out = arena.step(1_000_000, &[], &[switch]);
    assert_eq!(out.substeps, 0, "no time passed");
    let Some((_, after)) = out.frames.first().cloned() else {
        panic!("a switch alone sends the pilot a frame");
    };
    let Some(switched) = after.mind.as_ref() else {
        panic!("the mind");
    };
    assert_eq!(switched.active, 1);
    assert_eq!(
        switched.switch_at_ms,
        Some(arena.now_ms + crate::arena::intents::PROFILE_SWITCH_MS),
        "absolute: the switch's own millisecond plus ten seconds"
    );
    assert_eq!(after.pose.x_cm, before.pose.x_cm);
    assert_eq!(after.pose.heading_mdeg, before.pose.heading_mdeg);
    assert_eq!(after.speed_cms, before.speed_cms);
    assert!(!after.same_as(&before, 1_000), "the switch is told");
    let unswitched = Frame {
        mind: before.mind.clone(),
        ..after.clone()
    };
    assert!(
        unswitched.same_as(&before, 1_000),
        "and nothing but the mind differs"
    );
    assert!(
        arena.step(1_000_000, &[], &[]).frames.is_empty(),
        "the next-switch moment does not move with the clock"
    );
}

/// **17b cut M5.4** (doc 17 §12, §9; the M5 design note's M5.4 item 2):
/// the frame's classes are read LIVE. A heavy's hull, its core Class III,
/// rated and sealed at Class III under the fixture terms (their cap,
/// `fill_class`, raised to III), reads `class` III on a `core_class` III;
/// then its core is degraded one class by `degrade_core` — the op built by
/// `Arena::core_ops`, the builder a destroyed section's plan takes it from
/// (one `AdjustCell { core.core_class, -1 }`), applied as a plan, and the
/// consequence applied — and the frame reads `core_class` II and `class`
/// II — the seal still records III — and [`Frame::same_as`] is false
/// across the degrade, so the pilot is told, the mind alone differing.
#[test]
fn m5_the_frames_mind_reads_the_core_class_now() {
    use crate::arena::core::CoreClass;
    use crate::arena::test_support::tactical_unit_cored;
    let heavy = crate::arena::state::CustodyUnit {
        unit: tactical_unit_cored(A, CoreClass::III),
        ..tank(A, PartyId::RED, pose(3_500, 3_500))
    };
    let terms = crate::match_::SealTerms::fixture().rated(CoreClass::III);
    assert_eq!(terms.class_cap, CoreClass::III, "the raised cap");
    let heavy = m5_sealed(heavy, &["Line"], terms);
    assert_eq!(heavy.seal.class, CoreClass::III, "sealed at III");
    let mut arena = arena_at_epoch(vec![heavy], 1);
    let Some(before) = arena.frame_for(A, Tempo::Quiet) else {
        panic!("a frame");
    };
    let Some(mind) = before.mind.as_ref() else {
        panic!("its mind");
    };
    assert_eq!(
        (mind.class, mind.core_class),
        (CoreClass::III, CoreClass::III)
    );

    let Some(core) = arena.core_of(A) else {
        panic!("the heavy's core");
    };
    let addr = mush_tactics::CellAddr::Component {
        component: mush_tactics::ComponentId::new(core.component.as_str())
            .unwrap_or_else(|_| panic!("a component id")),
        cell: mush_tactics::CellId::new(crate::definitions::unit::CORE_CLASS_CELL)
            .unwrap_or_else(|_| panic!("a cell id")),
    };
    // `degrade_core`'s op, as a destroyed section's plan takes it.
    let ops = arena.core_ops(A, &[crate::arena::resolution::Consequence::DegradeCore]);
    assert_eq!(
        ops,
        [mush_tactics::UnitMutationOp::AdjustCell { addr, delta: -1 }],
        "one class off the core's own cell"
    );
    let plan = mush_tactics::UnitMutationPlan::new(ops, None, None)
        .unwrap_or_else(|_| panic!("the plan fits"));
    let Some(unit) = arena.unit(A).map(|state| state.unit.clone()) else {
        panic!("the heavy");
    };
    let (post, _) = mush_tactics::apply(&unit, &plan, 0)
        .unwrap_or_else(|error| panic!("the plan applies: {error}"));
    arena.set_unit(A, post, Arena::plan_touches_bands(&plan));
    let mut journal = Vec::new();
    arena.apply_consequence(
        A,
        &crate::arena::resolution::Consequence::DegradeCore,
        0,
        None,
        &mut journal,
    );

    let Some(after) = arena.frame_for(A, Tempo::Quiet) else {
        panic!("a frame");
    };
    let Some(degraded) = after.mind.as_ref() else {
        panic!("its mind");
    };
    assert_eq!(degraded.core_class, CoreClass::II, "the core as it is now");
    assert_eq!(degraded.class, CoreClass::II, "the mind thinks at it");
    assert_eq!(
        arena.unit(A).map(|state| state.seal.class),
        Some(CoreClass::III),
        "the seal keeps the sealed figure"
    );
    assert!(!after.same_as(&before, 1_000), "the degrade is told");
    let undegraded = Frame {
        mind: before.mind.clone(),
        ..after.clone()
    };
    assert!(
        undegraded.same_as(&before, 1_000),
        "and nothing but the mind differs"
    );
}

/// **16c `T-105`**: the chassis table on the arena is the unit's own two
/// functions over its own chassis and profile — one row per unit in the
/// unit table's order, its mask the one `fatal_sections` was laid down
/// from — and `frame_scope` reads the table, not the functions: a doctored
/// row moves the frame's fatal words, its `reads_dead` and its scan
/// table. A snapshot re-derives the honest table, whatever the arena held.
#[test]
fn t105_the_chassis_table_is_the_units_own_words_and_the_frame_reads_it() {
    use crate::arena::snapshot::Snapshot;
    use crate::arena::state::{ChassisWords, fatal_section_mask, fatal_section_words};
    use m1a3::*;
    let mut arena = field();
    assert_eq!(arena.chassis.len(), arena.units.len());
    for (at, unit) in arena.units.iter().enumerate() {
        let words = arena.chassis.get(at).expect("a row per unit");
        assert_eq!(arena.unit_at(unit.id), Some(at));
        assert_eq!(
            words.fatal_mask,
            fatal_section_mask(&unit.unit, &unit.profile)
        );
        assert_eq!(words.fatal_mask, unit.fatal_sections, "unit {}", unit.id.0);
        assert_eq!(
            words.fatal_words,
            fatal_section_words(&unit.unit, &unit.profile)
        );
        let names: Vec<SmolStr> = unit
            .unit
            .sections()
            .map(|(section, _)| SmolStr::new(section.as_str()))
            .collect();
        assert_eq!(words.section_names, names);
        assert_eq!(*words, ChassisWords::of(&unit.unit, &unit.profile));
    }

    // The frame reads the table: TIE's row, given a reading, carries the
    // chassis's fatal words off it and reads dead off its mask.
    if let Some(held) = arena.contacts.get_mut(RED, TIE_ID) {
        held.seen_condition = Some(reading_with(1));
    }
    let honest = row(&red(&arena), TIE_ID);
    assert_eq!(honest.fatal, Some(vec![SmolStr::new_static("hull")]));
    assert_eq!(honest.reads_dead, Some(true));
    assert!(!arena.frame_scan_tables(RED).is_empty());
    let tie_target = arena
        .contacts
        .of(RED)
        .find(|(id, _)| **id == TIE_ID)
        .map(|(_, held)| held.target)
        .expect("TIE is held");
    let tie_at = arena.unit_at(tie_target).expect("TIE's hull");
    let mut doctored: Vec<ChassisWords> = arena.chassis.to_vec();
    doctored[tie_at] = ChassisWords {
        fatal_mask: 0,
        fatal_words: vec![SmolStr::new_static("turret")],
        section_names: vec![SmolStr::new_static("turret")],
        unit_type: SmolStr::new_static("tacsim.unit.line_tank"),
    };
    arena.chassis = doctored.into();
    let read = row(&red(&arena), TIE_ID);
    assert_eq!(
        read.fatal,
        Some(vec![SmolStr::new_static("turret")]),
        "the words are the table's"
    );
    assert_eq!(read.reads_dead, Some(false), "the mask is the table's");
    assert!(
        arena
            .frame_scan_tables(RED)
            .iter()
            .any(|table| table.as_slice() == [SmolStr::new_static("turret")]),
        "the scan table is the table's"
    );

    // The snapshot re-derives the honest table.
    let snapshot = Snapshot::encode(&arena).expect("an arena between pulses encodes");
    let decoded = Snapshot::decode(&snapshot.document, (*arena.definition).clone())
        .expect("the snapshot decodes");
    assert_eq!(decoded.chassis.len(), decoded.units.len());
    for (at, unit) in decoded.units.iter().enumerate() {
        assert_eq!(
            decoded.chassis.get(at),
            Some(&ChassisWords::of(&unit.unit, &unit.profile))
        );
    }
    assert_eq!(
        row(&red(&decoded), TIE_ID).fatal,
        Some(vec![SmolStr::new_static("hull")])
    );
}

/// **16c `T-104`, the walks**: the tail phase of a wake hashes the state
/// only when it journals a line — a quiet wake takes no hash there at
/// all, and a wake that journals the tempo and the floor takes one, the
/// hash both lines carry, equal to the state's hash at that moment.
#[test]
fn t104a_the_wakes_tail_phase_hashes_only_when_it_journals() {
    let taken = || crate::arena::hash::STATE_HASHES_TAKEN.with(std::cell::Cell::get);
    let mut arena = arena();
    let mut journal = Log::default();
    // The opening pulse runs no substep and journals nothing: no hash.
    let before = taken();
    journal.absorb(&arena.step(0, &[], &[]));
    assert_eq!(taken() - before, 0, "a quiet wake walks nothing");
    // A wake that raises the tempo and sets a floor journals two lines
    // under one hash: the tail phase walks once for both.
    let fire = Intent {
        unit: A,
        channel: Channel::Fire,
        epoch: 1,
        kind: IntentKind::Fire {
            weapon: SmolStr::new("cannon"),
            slot: None,
        },
        seq: 2,
    };
    arena.clock.set_floor(Some(5_000));
    journal.absorb(&arena.step(5_000, &[], &[fire]));
    let log = journal.events(&mut arena);
    let tempo = log
        .iter()
        .find(|event| matches!(event.kind, EventKind::Tempo { .. }))
        .expect("the fire raised the tempo");
    let floor = log
        .iter()
        .find(|event| matches!(event.kind, EventKind::TempoFloor { .. }))
        .expect("the floor line");
    assert_eq!(tempo.hash, floor.hash, "one hash for the phase's lines");
    assert_eq!(tempo.t_ms, floor.t_ms);
    // And it is the hash the phase's moment has: the ledger's sequence
    // is folded, so the hash on the lines is the state's hash before the
    // first of them was appended — the eager walk's value, line for line.
    assert_ne!(tempo.hash, 0);
}

/// **16c T-110a: the reference** — `Arena::frame_sections`' body as it was
/// before the cut, word for word: the two ids built by `CellId::new` on
/// every call, one `probe` per section for its internals and one per facet
/// for its armour, each address built from cloned ids. Kept here so the
/// merged walk is held to the probe walk and not to a figure somebody
/// typed; `probe` is `TacticalUnit::cell` on a unit and `BTreeMap::get` on
/// a hand-built map, which is the same read.
fn lookup_walk<'a>(
    sections: impl Iterator<Item = (&'a SectionId, &'a SectionState)>,
    probe: impl Fn(&CellAddr) -> Option<&'a ScalarCell>,
) -> Vec<FrameSection> {
    let Ok(armor_cell) = CellId::new(crate::definitions::unit::ARMOR_CELL) else {
        return Vec::new();
    };
    let Ok(internals_cell) = CellId::new(crate::definitions::unit::INTERNALS_CELL) else {
        return Vec::new();
    };
    sections
        .map(|(id, section)| FrameSection {
            id: SmolStr::new(id.as_str()),
            condition: SmolStr::new(section.condition.as_str()),
            internals: probe(&CellAddr::Section {
                section: id.clone(),
                cell: internals_cell.clone(),
            })
            .map(|cell| (cell.current, cell.max)),
            facets: section
                .facets
                .iter()
                .filter_map(|facet| {
                    let cell = probe(&CellAddr::Facet {
                        section: id.clone(),
                        facet: facet.clone(),
                        cell: armor_cell.clone(),
                    })?;
                    Some(FrameFacet {
                        id: SmolStr::new(facet.as_str()),
                        armor: cell.current,
                        armor_max: cell.max,
                    })
                })
                .collect(),
        })
        .collect()
}

/// The block's rows as bare text and numbers, for an assertion that reads.
type Shape<'a> = Vec<(
    &'a str,
    &'a str,
    Option<(i64, i64)>,
    Vec<(&'a str, i64, i64)>,
)>;

fn shape(sections: &[FrameSection]) -> Shape<'_> {
    sections
        .iter()
        .map(|section| {
            (
                section.id.as_str(),
                section.condition.as_str(),
                section.internals,
                section
                    .facets
                    .iter()
                    .map(|facet| (facet.id.as_str(), facet.armor, facet.armor_max))
                    .collect(),
            )
        })
        .collect()
}

fn section_id(name: &str) -> SectionId {
    SectionId::new(name).unwrap_or_else(|why| panic!("{name}: {why:?}"))
}

fn facet_id(name: &str) -> FacetId {
    FacetId::new(name).unwrap_or_else(|why| panic!("{name}: {why:?}"))
}

fn cell_id(name: &str) -> CellId {
    CellId::new(name).unwrap_or_else(|why| panic!("{name}: {why:?}"))
}

/// **16c T-110a: the once-built ids are the constants** — the same text
/// `CellId::new` answers for them, and one allocation per process: every
/// read hands back the same reference.
#[test]
fn t110a_the_once_built_ids_are_the_constants() {
    assert_eq!(
        armor_cell().map(CellId::as_str),
        Some(crate::definitions::unit::ARMOR_CELL)
    );
    assert_eq!(
        internals_cell().map(CellId::as_str),
        Some(crate::definitions::unit::INTERNALS_CELL)
    );
    assert_eq!(
        armor_cell(),
        CellId::new(crate::definitions::unit::ARMOR_CELL)
            .ok()
            .as_ref()
    );
    assert_eq!(
        internals_cell(),
        CellId::new(crate::definitions::unit::INTERNALS_CELL)
            .ok()
            .as_ref()
    );
    assert!(
        armor_cell()
            .zip(armor_cell())
            .is_some_and(|(first, again)| std::ptr::eq(first, again)),
        "built once"
    );
    assert!(
        internals_cell()
            .zip(internals_cell())
            .is_some_and(|(first, again)| std::ptr::eq(first, again)),
        "built once"
    );
}

/// **16c T-110a: the cursors' rank is the address's order** — `rank_section`
/// and `rank_facet` against `CellAddr::cmp` on the address they stand in
/// for, over every cell of the stock tank against every target: each of
/// its sections and two it does not name (one sorting before them all, one
/// after), each of its facets and two it does not name, and four cell ids
/// (the two the walk asks for, one sorting between them, one after). All
/// three orderings occur, and `Equal` exactly as often as the map holds a
/// matching entry — sixteen on the stock tank, its five internals and
/// eleven armour cells.
#[test]
fn t110a_the_cursor_ranks_as_the_address_orders() {
    let unit = crate::arena::test_support::line_tank_unit(A);
    let sections: Vec<SectionId> = unit
        .sections()
        .map(|(id, _)| id.clone())
        .chain([section_id("aft"), section_id("zz_mast")])
        .collect();
    let facets: Vec<FacetId> = unit
        .sections()
        .flat_map(|(_, section)| section.facets.iter().cloned())
        .collect::<std::collections::BTreeSet<FacetId>>()
        .into_iter()
        .chain([facet_id("aft"), facet_id("zz")])
        .collect();
    let cells: Vec<CellId> = ["armor", "heat", "internals", "zzz"]
        .into_iter()
        .map(cell_id)
        .collect();
    let slot = |ordering: std::cmp::Ordering| match ordering {
        std::cmp::Ordering::Less => 0,
        std::cmp::Ordering::Equal => 1,
        std::cmp::Ordering::Greater => 2,
    };
    let mut seen = [0_usize; 3];
    for (addr, _) in unit.cells() {
        for section in &sections {
            for cell in &cells {
                let target = CellAddr::Section {
                    section: section.clone(),
                    cell: cell.clone(),
                };
                let want = addr.cmp(&target);
                assert_eq!(
                    rank_section(addr, section, cell),
                    want,
                    "{addr:?} against {target:?}"
                );
                seen[slot(want)] += 1;
                for facet in &facets {
                    let target = CellAddr::Facet {
                        section: section.clone(),
                        facet: facet.clone(),
                        cell: cell.clone(),
                    };
                    let want = addr.cmp(&target);
                    assert_eq!(
                        rank_facet(addr, section, facet, cell),
                        want,
                        "{addr:?} against {target:?}"
                    );
                    seen[slot(want)] += 1;
                }
            }
        }
    }
    assert!(
        seen.iter().all(|n| *n > 0),
        "every ordering occurs: {seen:?}"
    );
    let matching = unit
        .cells()
        .filter(|(addr, _)| match addr {
            CellAddr::Section { cell, .. } => cells.contains(cell),
            CellAddr::Facet { facet, cell, .. } => facets.contains(facet) && cells.contains(cell),
            CellAddr::Unit { .. } | CellAddr::Component { .. } => false,
        })
        .count();
    assert_eq!(seen[1], matching, "one Equal per matching entry");
    assert_eq!(
        matching, 16,
        "the stock tank's five internals and eleven plates"
    );
}

/// **16c T-110a: a cursor stops short of its target and keeps the entry**
/// — `take_at`'s three arms by hand, on a run of two section entries,
/// `b/internals` and `d/internals`. `a` ends `None` with nothing consumed;
/// `c` consumes `b` (ranked `Less`), stops on `d` (ranked `Greater`) and
/// answers `None` with `d` still the head of the run; `d` then answers its
/// own cell, which a cursor that consumed the entry it stopped at could
/// not; `e` runs the cursor out. A fresh run takes `b` first, so the
/// `Greater` stop is also tried straight after an `Equal` take.
#[test]
fn t110a_a_cursor_stops_short_and_keeps_the_entry() {
    type Run<'a> = std::iter::Peekable<std::collections::btree_map::Iter<'a, CellAddr, ScalarCell>>;
    fn ask(run: &mut Run<'_>, section: &str, internals: &CellId) -> Option<i64> {
        let section = section_id(section);
        take_at(run, |addr| rank_section(addr, &section, internals)).map(|cell| cell.current)
    }
    fn head(run: &mut Run<'_>) -> Option<CellAddr> {
        run.peek().map(|(addr, _)| (*addr).clone())
    }
    let Some(internals) = internals_cell() else {
        panic!("the once-built id");
    };
    let at = |section: &str| CellAddr::Section {
        section: section_id(section),
        cell: cell_id("internals"),
    };
    let scalar = |current: i64| ScalarCell {
        min: 0,
        max: 9,
        current,
    };
    let cells: BTreeMap<CellAddr, ScalarCell> = [(at("b"), scalar(2)), (at("d"), scalar(4))]
        .into_iter()
        .collect();

    let mut run = cells.iter().peekable();
    assert_eq!(ask(&mut run, "a", internals), None, "a: before the run");
    assert_eq!(head(&mut run), Some(at("b")), "a consumed nothing");
    assert_eq!(ask(&mut run, "c", internals), None, "c: between b and d");
    assert_eq!(head(&mut run), Some(at("d")), "c stopped on d and kept it");
    assert_eq!(ask(&mut run, "d", internals), Some(4), "d: the kept entry");
    assert_eq!(head(&mut run), None, "the run is spent");
    assert_eq!(ask(&mut run, "e", internals), None, "e: past the end");

    let mut run = cells.iter().peekable();
    assert_eq!(ask(&mut run, "b", internals), Some(2), "b: the first entry");
    assert_eq!(head(&mut run), Some(at("d")), "b consumed only b");
    assert_eq!(
        ask(&mut run, "c", internals),
        None,
        "c: straight after a take"
    );
    assert_eq!(head(&mut run), Some(at("d")), "c kept d");
    assert_eq!(ask(&mut run, "d", internals), Some(4));
}

/// **16c T-110a: the merge is the probe walk on maps built by hand** — the
/// three shapes the shipped content never builds, fed as the very types
/// `TacticalUnit::sections` and `TacticalUnit::cells` iterate: a facet
/// with no armour cell (`aft/right`, which has a `paint` cell and nothing
/// else, dropped as the probe walk drops it), a section with no internals
/// cell (`bow`, `None` as the probe walk reads it), and `Facet` and
/// `Section` entries for a facet (`aft/stern`) and a section (`keel`) the
/// sections do not name, which both walks pass over. Beside them the cells
/// a cursor must step across — a `Unit` pool, section cells sorting on
/// either side of `internals`, a second cell on a plated facet, a
/// `Component` magazine — and `zed`, whose one plate lies past `keel`'s so
/// the facet cursor skips a foreign section's entry to reach it.
///
/// Two misses stop on the next target's own entry, one on each cursor:
/// `lamp` has no cell at all and its internals probe stops on
/// `mast/internals`; `bow/rear` has no cell and its plate probe stops on
/// `bow/top/armor`. The shipped content never reaches `take_at`'s
/// `Greater` arm — the line tank's section run is exactly its five
/// `internals`, and every facet cell is `armor` — so these two are what
/// hold the arm to *stop without consuming*: a cursor that consumed the
/// entry it stopped at would read `mast` as `None` and drop `bow`'s top,
/// and the equality and the shape below both fail it. Then a run that
/// ends before the target, so the cursor's end is the probe's miss.
///
/// The maps are built by hand because `TacticalUnit` refuses them: the
/// stock tank's own document with a plate on a facet its section does not
/// name, or an internals cell on a section it does not have, fails to
/// decode — asserted here, which is why the filters cannot be exercised
/// through the unit and must be exercised through its iterators.
#[test]
fn t110a_the_merge_is_the_lookup_walk_on_hand_built_maps() {
    let (Some(armor), Some(internals)) = (armor_cell(), internals_cell()) else {
        panic!("the once-built ids");
    };
    let scalar = |current: i64, max: i64| ScalarCell {
        min: 0,
        max,
        current,
    };
    let state = |condition: &str, facets: &[&str]| SectionState {
        condition: mush_tactics::ConditionId::new(condition)
            .unwrap_or_else(|why| panic!("{condition}: {why:?}")),
        facets: facets.iter().map(|facet| facet_id(facet)).collect(),
        slots: std::collections::BTreeSet::new(),
    };
    let at = |section: &str, cell: &str| CellAddr::Section {
        section: section_id(section),
        cell: cell_id(cell),
    };
    let on = |section: &str, facet: &str, cell: &str| CellAddr::Facet {
        section: section_id(section),
        facet: facet_id(facet),
        cell: cell_id(cell),
    };
    let sections: BTreeMap<SectionId, SectionState> = [
        (section_id("aft"), state("intact", &["left", "right"])),
        (
            section_id("bow"),
            state("destroyed", &["front", "rear", "top"]),
        ),
        (section_id("lamp"), state("intact", &[])),
        (section_id("mast"), state("intact", &[])),
        (section_id("zed"), state("intact", &["x"])),
    ]
    .into_iter()
    .collect();
    let cells: BTreeMap<CellAddr, ScalarCell> = [
        (
            CellAddr::Unit {
                cell: cell_id("heat"),
            },
            scalar(3, 9),
        ),
        (at("aft", "heat"), scalar(1, 5)),
        (at("aft", "internals"), scalar(40, 50)),
        (at("aft", "speed"), scalar(2, 2)),
        (at("bow", "heat"), scalar(0, 5)),
        (at("keel", "internals"), scalar(7, 7)),
        (at("mast", "internals"), scalar(0, 30)),
        (on("aft", "left", "armor"), scalar(12, 20)),
        (on("aft", "right", "paint"), scalar(1, 1)),
        (on("aft", "stern", "armor"), scalar(9, 9)),
        (on("bow", "front", "armor"), scalar(0, 25)),
        (on("bow", "front", "plating"), scalar(1, 1)),
        (on("bow", "top", "armor"), scalar(25, 25)),
        (on("keel", "left", "armor"), scalar(5, 5)),
        (on("zed", "x", "armor"), scalar(4, 8)),
        (
            CellAddr::Component {
                component: mush_tactics::ComponentId::new("gun")
                    .unwrap_or_else(|why| panic!("gun: {why:?}")),
                cell: cell_id("ammo"),
            },
            scalar(3, 40),
        ),
    ]
    .into_iter()
    .collect();
    let merged = frame_sections_merged(
        sections.iter(),
        cells.iter(),
        cells.iter(),
        armor,
        internals,
    );
    let walked = lookup_walk(sections.iter(), |addr| cells.get(addr));
    assert_eq!(merged, walked);
    assert_eq!(
        shape(&merged),
        vec![
            ("aft", "intact", Some((40, 50)), vec![("left", 12, 20)]),
            (
                "bow",
                "destroyed",
                None,
                vec![("front", 0, 25), ("top", 25, 25)]
            ),
            ("lamp", "intact", None, Vec::new()),
            ("mast", "intact", Some((0, 30)), Vec::new()),
            ("zed", "intact", None, vec![("x", 4, 8)]),
        ],
        "the shape, so the equality is not two empty lists agreeing"
    );

    // A run that ends before the target: one internals cell and no plate
    // at all, under two sections — the section cursor runs out before
    // `bow`'s target and the facet cursor before `aft/left`'s.
    let short: BTreeMap<SectionId, SectionState> = [
        (section_id("aft"), state("intact", &["left"])),
        (section_id("bow"), state("intact", &[])),
    ]
    .into_iter()
    .collect();
    let one: BTreeMap<CellAddr, ScalarCell> = [(at("aft", "internals"), scalar(6, 6))]
        .into_iter()
        .collect();
    let merged = frame_sections_merged(short.iter(), one.iter(), one.iter(), armor, internals);
    assert_eq!(merged, lookup_walk(short.iter(), |addr| one.get(addr)));
    assert_eq!(
        shape(&merged),
        vec![
            ("aft", "intact", Some((6, 6)), Vec::new()),
            ("bow", "intact", None, Vec::new()),
        ]
    );

    // And why by hand: the unit refuses both unnamed addresses on load,
    // where its own document decodes.
    let unit = crate::arena::test_support::line_tank_unit(A);
    let doc = serde_json::to_value(&unit).expect("the stock tank encodes");
    assert_eq!(
        serde_json::from_value::<mush_tactics::TacticalUnit>(doc.clone()).ok(),
        Some(unit),
        "the stock tank's own document decodes to itself"
    );
    for (what, addr) in [
        (
            "a plate on a facet the hull does not name",
            serde_json::json!({"Facet": {"section": "hull", "facet": "stern", "cell": "armor"}}),
        ),
        (
            "internals on a section the tank does not have",
            serde_json::json!({"Section": {"section": "keel", "cell": "internals"}}),
        ),
    ] {
        let mut doc = doc.clone();
        let Some(cells) = doc
            .get_mut("cells")
            .and_then(serde_json::Value::as_array_mut)
        else {
            panic!("the document's cells");
        };
        cells.push(serde_json::json!([
            addr,
            {"min": 0, "max": 1, "current": 1}
        ]));
        assert!(
            serde_json::from_value::<mush_tactics::TacticalUnit>(doc).is_err(),
            "{what} decodes"
        );
    }
}

/// **16c T-110a: the stock chassis, whole and at its floors, walks the
/// same** — two line tanks under seated pilots, every unit's block held
/// to the probe walk whole; then red struck by one plan to the shapes a
/// fight reaches (the hull's rear plate and the engine's internals at
/// their floors, the turret's front and the hull's internals part way,
/// the engine `destroyed`) and held again, with the figures read back so
/// the equality is over a struck tank; then four pulses of the arena
/// with the struck tank in it, every unit and every frame the pilots were
/// sent held to the walk over the cells as they stand.
#[test]
fn t110a_the_stock_chassis_at_its_floors_walks_the_same() {
    use crate::arena::test_support::line_tank;
    const B: ObjId = ObjId(9);
    let same = |arena: &Arena| {
        for state in &arena.units {
            let walked = lookup_walk(state.unit.sections(), |addr| state.unit.cell(addr));
            assert_eq!(arena.frame_sections(state.id), walked, "#{}", state.id.0);
            assert_eq!(
                Arena::frame_sections_of(&state.unit),
                walked,
                "#{} off the unit in hand",
                state.id.0
            );
        }
    };
    let mut arena = arena_at_epoch(
        vec![
            line_tank(A, PartyId::RED, pose(1_500, 1_500)),
            line_tank(B, PartyId::BLUE, pose(1_500, 2_500)),
        ],
        1,
    );
    same(&arena);
    let whole = arena.frame_sections(A);
    assert_eq!(
        whole
            .iter()
            .map(|section| section.id.as_str())
            .collect::<Vec<&str>>(),
        vec!["engine", "hull", "left_track", "right_track", "turret"]
    );
    let set = |addr: CellAddr, value: i64| mush_tactics::UnitMutationOp::SetCell { addr, value };
    let plan = mush_tactics::UnitMutationPlan::new(
        vec![
            set(
                CellAddr::Facet {
                    section: section_id("hull"),
                    facet: facet_id("rear"),
                    cell: cell_id("armor"),
                },
                0,
            ),
            set(
                CellAddr::Section {
                    section: section_id("engine"),
                    cell: cell_id("internals"),
                },
                0,
            ),
            set(
                CellAddr::Facet {
                    section: section_id("turret"),
                    facet: facet_id("front"),
                    cell: cell_id("armor"),
                },
                123,
            ),
            set(
                CellAddr::Section {
                    section: section_id("hull"),
                    cell: cell_id("internals"),
                },
                77,
            ),
            mush_tactics::UnitMutationOp::SetSectionCondition {
                section: section_id("engine"),
                condition: mush_tactics::ConditionId::new(crate::arena::damage::DESTROYED)
                    .unwrap_or_else(|why| panic!("destroyed: {why:?}")),
            },
        ],
        None,
        None,
    )
    .unwrap_or_else(|why| panic!("a plan: {why:?}"));
    if let Some(state) = arena.unit_mut(A) {
        state.unit = mush_tactics::apply(&state.unit, &plan, 0)
            .unwrap_or_else(|why| panic!("the plan applies: {why:?}"))
            .0;
    }
    same(&arena);
    let struck = arena.frame_sections(A);
    assert_ne!(struck, whole, "the plan moved the block");
    assert_eq!(
        shape(&struck),
        vec![
            ("engine", "destroyed", Some((0, 120)), Vec::new()),
            (
                "hull",
                "intact",
                Some((77, 200)),
                vec![
                    ("front", 400, 400),
                    ("left", 250, 250),
                    ("rear", 0, 150),
                    ("right", 250, 250),
                    ("top", 100, 100),
                ]
            ),
            (
                "left_track",
                "intact",
                Some((100, 100)),
                vec![("left", 120, 120)]
            ),
            (
                "right_track",
                "intact",
                Some((100, 100)),
                vec![("right", 120, 120)]
            ),
            (
                "turret",
                "intact",
                Some((150, 150)),
                vec![
                    ("front", 123, 350),
                    ("left", 220, 220),
                    ("rear", 140, 140),
                    ("right", 220, 220),
                ]
            ),
        ]
    );
    let mut wall = 0_i64;
    let mut frames = 0_usize;
    for _ in 0..4 {
        let out = arena.step(wall, &[], &[]);
        same(&arena);
        for (_, frame) in &out.frames {
            let Some(state) = arena.unit(frame.unit) else {
                panic!("#{} is in the match", frame.unit.0);
            };
            assert_eq!(
                frame.sections,
                lookup_walk(state.unit.sections(), |addr| state.unit.cell(addr)),
                "the frame sent for #{}",
                frame.unit.0
            );
            frames += 1;
        }
        wall += 500;
    }
    assert!(frames >= 2, "the pilots were sent frames: {frames}");
    assert_eq!(arena.frame_sections(A), struck, "nothing else moved it");
}

fn m6_assert_pulse_outputs(a: &StepOutput, b: &StepOutput) {
    assert_eq!(
        (a.tempo, a.next_due_ms, a.lagged, a.ended, a.substeps),
        (b.tempo, b.next_due_ms, b.lagged, b.ended, b.substeps)
    );
    assert_eq!(a.perception, b.perception);
    assert_eq!(a.motor, b.motor);
    assert_eq!(format!("{:?}", a.frames), format!("{:?}", b.frames));
    assert_eq!(format!("{:?}", a.observer), format!("{:?}", b.observer));
    assert_eq!(
        format!("{:?}", a.deliberation),
        format!("{:?}", b.deliberation)
    );
    assert_eq!(
        a.ledger_chunks.iter().map(|c| &c.bytes).collect::<Vec<_>>(),
        b.ledger_chunks.iter().map(|c| &c.bytes).collect::<Vec<_>>()
    );
    assert_eq!(
        a.thinks
            .iter()
            .map(|t| (t.unit, t.think_seq, t.t_ms, t.lands_ms))
            .collect::<Vec<_>>(),
        b.thinks
            .iter()
            .map(|t| (t.unit, t.think_seq, t.t_ms, t.lands_ms))
            .collect::<Vec<_>>()
    );
}
fn m6_sliced(
    arena: Arena,
    wall: i64,
    controls: &[Control],
    intents: &[Intent],
) -> (Arena, StepOutput, usize) {
    let mut pulse = OwnedPulse::new(arena, wall, controls, intents);
    let mut turns = 0;
    loop {
        let reserved = pulse.next_simulation_ms();
        let progress = pulse.advance();
        turns += 1;
        assert_eq!(
            progress.simulated_ms, reserved,
            "each turn advances exactly its promised native substep"
        );
        assert!(turns < 1000, "a bounded pulse finishes");
        if progress.done {
            break;
        }
    }
    let (arena, output) = pulse
        .finish()
        .unwrap_or_else(|_| panic!("completed private pulse"));
    (arena, output, turns)
}
#[test]
fn m6_owned_pulse_matches_live_zero_one_many_phases_and_profile_change() {
    let make = || {
        arena_at_epoch(
            vec![m5_sealed(
                tank(A, PartyId::RED, pose(3_500, 3_500)),
                &["Sniper", "Line"],
                crate::match_::SealTerms::fixture(),
            )],
            1,
        )
    };
    let mut live = make();
    let mut sliced = make();
    for wall in [0, 0, 250, 5_250, 5_250] {
        let intents = if wall == 250 {
            vec![Intent {
                unit: A,
                channel: Channel::Helm,
                epoch: 1,
                kind: IntentKind::Profile { index: 1 },
                seq: 1,
            }]
        } else {
            vec![]
        };
        let output = live.step(wall, &[], &intents);
        let (next, actual, turns) = m6_sliced(sliced, wall, &[], &intents);
        sliced = next;
        m6_assert_pulse_outputs(&output, &actual);
        assert_eq!(
            crate::arena::state_hash(&live),
            crate::arena::state_hash(&sliced)
        );
        assert_eq!(live.substep_hashes, sliced.substep_hashes);
        assert!(
            turns >= 4,
            "integration, actions, drain, final record each yield"
        );
    }
    assert_eq!(live.profiles[&A].index, 1);
}
#[test]
fn m6_owned_pulse_matches_delayed_think_settlement_and_early_end() {
    use crate::arena::test_support::{arena_of, cannon};
    let make = || {
        let units = [(A, PartyId::RED, 1500), (ObjId(5), PartyId::BLUE, 6500)]
            .into_iter()
            .map(|(id, team, x)| {
                let mut unit = tank(id, team, pose(x, 3500));
                unit.controller = crate::arena::state::Controller::Mind;
                unit.weapons = vec![cannon()];
                unit.reseal(&crate::match_::SealTerms::fixture()).unwrap();
                unit
            })
            .collect();
        arena_of(units)
    };
    let mut live = make();
    let mut sliced = make();
    let mut pending = false;
    for wall in [0, 500, 1000, 1500, 5000] {
        let expected = live.step(wall, &[], &[]);
        pending |= !expected.thinks.is_empty();
        let (next, actual, _) = m6_sliced(sliced, wall, &[], &[]);
        sliced = next;
        m6_assert_pulse_outputs(&expected, &actual);
        assert_eq!(
            crate::arena::state_hash(&live),
            crate::arena::state_hash(&sliced)
        );
        // Deliberately leave collected work pending: the next pulse's first
        // phase must settle it once, with identical due/event ordering.
    }
    assert!(pending, "fixture exercises real deferred think jobs");
    live.stop(crate::arena::state::StopReason::Abort);
    sliced.stop(crate::arena::state::StopReason::Abort);
    let expected = live.step(5500, &[], &[]);
    let (sliced, actual, _) = m6_sliced(sliced, 5500, &[], &[]);
    m6_assert_pulse_outputs(&expected, &actual);
    assert!(actual.ended.is_some());
    assert_eq!(
        crate::arena::state_hash(&live),
        crate::arena::state_hash(&sliced)
    );
}
#[test]
fn m6_owned_pulse_matches_ledger_full_termination() {
    use crate::arena::test_support::custody;
    use crate::config::SensorsConfig;
    let make = || {
        let mut cfg = ArenaConfig::default();
        cfg.ledger.max_bytes = 1;
        Arena::new(
            custody(vec![tank(A, PartyId::RED, pose(3500, 3500))], vec![]),
            &cfg,
            &MatchConfig::default(),
            &SensorsConfig::default(),
        )
        .unwrap()
    };
    let mut live = make();
    let mut sliced = make();
    for wall in [0, 5000] {
        let expected = live.step(wall, &[], &[]);
        let (next, actual, _) = m6_sliced(sliced, wall, &[], &[]);
        sliced = next;
        m6_assert_pulse_outputs(&expected, &actual);
        assert_eq!(
            crate::arena::state_hash(&live),
            crate::arena::state_hash(&sliced)
        );
    }
    assert!(live.ended.is_some());
}

#[test]
fn m6_owned_pulse_stops_at_the_original_midpulse_deadline() {
    use crate::arena::test_support::custody;
    use crate::config::SensorsConfig;
    let make = || {
        Arena::new(
            custody(vec![tank(A, PartyId::RED, pose(3500, 3500))], vec![]),
            &ArenaConfig::default(),
            &MatchConfig {
                max_duration_ms: 1500,
                ..MatchConfig::default()
            },
            &SensorsConfig::default(),
        )
        .unwrap()
    };
    let mut live = make();
    let mut sliced = make();
    live.step(0, &[], &[]);
    sliced.step(0, &[], &[]);
    let expected = live.step(5000, &[], &[]);
    let (actual_arena, actual, turns) = m6_sliced(sliced, 5000, &[], &[]);
    assert_eq!(actual_arena.now_ms, 1500);
    assert_eq!(actual.substeps, 6);
    assert!(turns < 12, "no piece after the first terminal substep runs");
    assert_eq!(actual.ended.unwrap().t_ms, 1500);
    m6_assert_pulse_outputs(&expected, &actual);
    assert_eq!(live.substep_hashes, actual_arena.substep_hashes);
}

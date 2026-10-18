//! M6.6: two proving accounts share one production server with a live arena.
//! Public SSH commands cause every action. Durable reads assert resource/custody
//! facts only; no read is fed into a pilot or used to fabricate worker progress.
use serde_json::{Value, json};
use std::time::{Duration, Instant};

#[path = "support/arena.rs"]
mod rig;

const PROVE: &str = "@mind/prove Standard/Line VS Standard/Line 100 MUSTER 6v6 SEED 713";
const PROVING_GAMES: u64 = 100;
const PROVING_UNITS: u64 = 12;
const PROVING_LIMIT_MS: u64 = 600_000;
const PROVING_STEP_MS: u64 = 250;
// Cover Plan::maximum_unit_ms for the complete requested workload, including
// its final native step: 100 * 12 * (600_000 + 250) / 60_000 = 12_005.
// A valid fast first run must not stall on quota while the second prepares.
const DAILY_MINUTES: u64 =
    (PROVING_UNITS * (PROVING_LIMIT_MS + PROVING_STEP_MS) * PROVING_GAMES).div_ceil(60_000);
// Finite liveness guards for two complete 100-game custody/compression jobs on
// variable hosts. Measured latency stays evidence, not a performance threshold.
// Allow two full preparation guards plus 300 seconds for the functional checks.
const PREPARATION_TIMEOUT: Duration = Duration::from_secs(600);
const ISOLATION_TIMEOUT: Duration = Duration::from_secs(1_500);

async fn books(server: &rig::Rig) -> Vec<Value> {
    server
        .store
        .module_json_list("tacsim")
        .await
        .unwrap()
        .into_iter()
        .filter_map(|(key, value)| key.starts_with("tacsim:mindbulk:account:").then_some(value))
        .collect()
}
fn number(value: &Value, field: &str) -> u64 {
    value[field]
        .as_u64()
        .unwrap_or_else(|| panic!("required {field}: {value}"))
}
fn run(book: &Value) -> &Value {
    let rows = book["runs"].as_object().expect("bounded account runs");
    assert_eq!(rows.len(), 1, "refused duplicate creates no second run");
    rows.get("1").expect("first admitted run")
}
fn check_bounds(rows: &[Value]) {
    assert!(
        rows.len() <= 2,
        "the full queue cannot admit a third account"
    );
    for book in rows {
        let row = run(book);
        let plan = &row["plan"];
        assert_eq!(number(plan, "unit_count"), PROVING_UNITS);
        assert_eq!(number(plan, "games"), PROVING_GAMES);
        assert_eq!(number(plan, "limit_ms"), PROVING_LIMIT_MS);
        assert_eq!(number(plan, "native_substep_ms"), PROVING_STEP_MS);
        let plan_unit_ms = number(plan, "unit_count")
            * (number(plan, "limit_ms") + number(plan, "native_substep_ms"))
            * number(plan, "games");
        assert_eq!(
            plan_unit_ms.div_ceil(60_000),
            DAILY_MINUTES,
            "the quota covers the actual retained plan's complete workload"
        );
        assert!(
            number(book, "spent_unit_ms") + number(book, "reserved_unit_ms")
                <= DAILY_MINUTES * 60_000,
            "daily reserved+spent quota is bounded"
        );
        if let Some(flight) = row["flight"].as_object() {
            let allowance = flight["allowance_unit_ms"].as_u64().unwrap();
            let step = number(&row["plan"], "native_substep_ms");
            assert!(
                allowance <= 8 * step * 12,
                "one prepaid lease is at most eight native steps"
            );
        }
        if book["active"].is_null() {
            assert_eq!(number(book, "reserved_unit_ms"), 0);
        }
        assert!(number(row, "charged_unit_ms") <= DAILY_MINUTES * 60_000);
    }
}
fn settled_charge(book: &Value) -> u64 {
    let row = run(book);
    let prepaid = row["flight"]
        .as_object()
        .map_or(0, |flight| flight["allowance_unit_ms"].as_u64().unwrap());
    number(row, "charged_unit_ms")
        .checked_sub(prepaid)
        .expect("a current prepayment is part of the charged total")
}
async fn live_time(server: &rig::Rig, match_id: u64) -> Option<i64> {
    let value = server
        .store
        .module_json_get("tacsim", &format!("tacsim:snapshot:{match_id}"))
        .await
        .unwrap()?;
    let state: Value =
        serde_json::from_str(value["state"].as_str().expect("canonical snapshot")).unwrap();
    state["now_ms"].as_i64()
}
async fn require_live_time(
    server: &rig::Rig,
    pilot: &mut rig::Client,
    match_id: u64,
    phase: &str,
) -> i64 {
    if let Some(now) = live_time(server, match_id).await {
        return now;
    }
    pilot.pump(Duration::from_millis(100)).await;
    let result = server
        .store
        .module_json_get("tacsim", &format!("tacsim:match:{match_id}"))
        .await
        .unwrap();
    panic!(
        "{phase}: match {match_id} has no live snapshot; retained result={result:?}; pilot transcript:\n{}",
        pilot.seen
    );
}

async fn start_live(server: &rig::Rig, pilot: &mut rig::Client) -> u64 {
    pilot.say("SIM TEAM red", "takes red slot 1").await;
    pilot.say("SIM START", "is boarding").await;
    pilot.say("red", "Red Room").await;
    pilot.say("BOARD", "climb into").await;
    pilot.expect("Good hunting", rig::STEP_TIMEOUT).await;
    warm_cockpit(pilot).await;
    tokio::time::timeout(rig::STEP_TIMEOUT, async {
        let mut match_id = None;
        loop {
            if match_id.is_none() {
                let rows = server.store.module_json_list("tacsim").await.unwrap();
                let ids = rows
                    .iter()
                    .filter_map(|(key, _)| key.strip_prefix("tacsim:live:"))
                    .map(|id| id.parse::<u64>().expect("actual retained match identity"))
                    .collect::<Vec<_>>();
                assert!(ids.len() <= 1, "one live public sortie on this server");
                match_id = ids.first().copied();
            }
            if let Some(id) = match_id {
                // A public frame and live custody can precede deferred snapshot
                // persistence. Only the first durable snapshot starts measuring.
                if live_time(server, id).await.is_some() {
                    return id;
                }
                let result = server
                    .store
                    .module_json_get("tacsim", &format!("tacsim:match:{id}"))
                    .await
                    .unwrap();
                assert!(
                    result.is_none(),
                    "match {id} ended before its first durable snapshot; result={result:?}; pilot transcript:\n{}",
                    pilot.seen
                );
            }
            pilot.pump(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "public boarding produced no durable live snapshot:\n{}",
            pilot.seen
        )
    })
}

async fn close_baseline(server: &rig::Rig, pilot: &mut rig::Client, match_id: u64) -> Value {
    pilot.say("SIM CONCEDE", "concedes").await;
    let result = tokio::time::timeout(rig::STEP_TIMEOUT, async {
        loop {
            if let Some(result) = server
                .store
                .module_json_get("tacsim", &format!("tacsim:match:{match_id}"))
                .await
                .unwrap()
            {
                return result;
            }
            pilot.pump(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("baseline concession was not retained:\n{}", pilot.seen));
    assert!(
        live_time(server, match_id).await.is_none(),
        "baseline teardown completed before private preparation"
    );
    pilot.say("out", "Simulator Lobby").await;
    result
}

struct StatusSample {
    elapsed_us: u64,
    transient_retries: [usize; 4],
}
async fn warm_cockpit(pilot: &mut rig::Client) {
    let from = pilot.seen.len();
    // Good hunting precedes the asynchronous first cockpit frame. Retry that
    // explicit public not-ready answer; waiting on the same reply cannot help.
    tokio::time::timeout(rig::STEP_TIMEOUT, async {
        loop {
            let (answer, text) = pilot
                .answer(
                    "STATUS",
                    &["m north;", "The panel has told you nothing yet."],
                )
                .await;
            let warmup = &pilot.seen[from..];
            assert!(
                !warmup.contains("DESTROYED"),
                "the live pilot has a working hull: {warmup}"
            );
            match answer {
                Some(0) => break,
                Some(1) => tokio::time::sleep(Duration::from_millis(75)).await,
                _ => panic!("STATUS did not return a ready or explicitly pending panel: {text}"),
            }
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "first public cockpit frame did not arrive: {}",
            &pilot.seen[from..]
        )
    });
}
async fn retained_preparation(client: &mut rig::Client, from: usize, account: &str) {
    let deadline = tokio::time::Instant::now() + PREPARATION_TIMEOUT;
    loop {
        let text = &client.seen[from..];
        assert!(
            !text.contains("Proving preparation refused:"),
            "{account}'s proving preparation was explicitly refused:\n{text}"
        );
        if text.contains("Proving run 1 retained and queued.") {
            return;
        }
        assert!(
            !client.closed,
            "{account}'s SSH channel closed before proving admission:\n{text}"
        );
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        assert!(
            !left.is_zero(),
            "{account}'s proving preparation exceeded its liveness guard:\n{text}"
        );
        // Read without resubmitting PROVE. A short bounded read also notices
        // a refusal whose final newline arrives in a later SSH packet.
        client.pump(left.min(Duration::from_millis(100))).await;
    }
}
async fn status_sample(pilot: &mut rig::Client) -> StatusSample {
    let start = Instant::now();
    let from = pilot.seen.len();
    // A commit barrier can explicitly refuse this read. Use the same bounded
    // public retry contract as the other commands; include all waiting/retries
    // in the sample and retain their counts instead of reporting only the
    // eventual successful request's latency.
    pilot.say("STATUS", "m north;").await;
    let text = &pilot.seen[from..];
    assert!(
        !text.contains("DESTROYED"),
        "the live pilot still has a working hull: {text}"
    );
    StatusSample {
        elapsed_us: u64::try_from(start.elapsed().as_micros()).unwrap(),
        transient_retries: rig::TRANSIENTS.map(|needle| text.matches(needle).count()),
    }
}
fn timings(samples: &[StatusSample]) -> Value {
    assert!(!samples.is_empty());
    let elapsed = samples.iter().map(|s| s.elapsed_us).collect::<Vec<_>>();
    let retries = samples
        .iter()
        .map(|s| s.transient_retries)
        .collect::<Vec<_>>();
    let mut sorted = elapsed.clone();
    sorted.sort_unstable();
    json!({"samples_us":elapsed,"transient_kinds":rig::TRANSIENTS,
        "transient_retries":retries,"p50_us":sorted[(sorted.len()-1)/2],
        "p99_us":sorted[(sorted.len()*99).div_ceil(100)-1],"max_us":sorted.last()})
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_proving_accounts_share_one_worker_while_the_live_arena_keeps_progressing() {
    tokio::time::timeout(ISOLATION_TIMEOUT, async {
        let mut cfg = rig::mind_acceptance_config();
        assert_eq!(
            cfg.minds.proving_workers, 1,
            "exercise the shipped worker count"
        );
        cfg.minds.proving_queued = 1;
        cfg.minds.proving_daily_unit_minutes = DAILY_MINUTES;
        let server = rig::boot(
            cfg,
            &[
                ("live", "Pilot"),
                ("one", "One"),
                ("two", "Two"),
                ("overflow", "Overflow"),
            ],
        )
        .await;
        let mut pilot = rig::Client::login(&server, "live", rig::PILOT_PASSWORD, "Pilot").await;
        let mut one = rig::Client::login(&server, "one", rig::PILOT_PASSWORD, "One").await;
        let mut two = rig::Client::login(&server, "two", rig::PILOT_PASSWORD, "Two").await;
        let mut overflow =
            rig::Client::login(&server, "overflow", rig::PILOT_PASSWORD, "Overflow").await;
        rig::walk_to_lobby(&mut pilot).await;
        let baseline_match = start_live(&server, &mut pilot).await;
        let mut baseline = Vec::new();
        for _ in 0..16 {
            baseline.push(status_sample(&mut pilot).await);
            tokio::time::sleep(Duration::from_millis(75)).await;
        }
        assert!(
            books(&server).await.is_empty(),
            "baseline has no private work"
        );
        let before = require_live_time(&server, &mut pilot, baseline_match, "baseline").await;
        // Preparation is not the workload under measurement. Close the normal
        // baseline sortie publicly, then open a fresh normal sortie once both
        // jobs are admitted, so two long preparations cannot consume its life.
        let baseline_result = close_baseline(&server, &mut pilot, baseline_match).await;

        let one_preparing = Instant::now();
        let one_from = one.seen.len();
        one.say(PROVE, "Proving preparation queued").await;
        retained_preparation(&mut one, one_from, "one").await;
        let one_preparation_ms = u64::try_from(one_preparing.elapsed().as_millis()).unwrap();
        println!(
            "M6_LIVE_PROVING_PREPARATION {}",
            json!({"account":"one", "elapsed_ms":one_preparation_ms})
        );
        // The second job must enter behind an already-running first account,
        // not race its preparation into the single queued slot.
        loop {
            let rows = books(&server).await;
            check_bounds(&rows);
            if rows.len() == 1 && number(run(&rows[0]), "next_sequence") >= 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let two_preparing = Instant::now();
        let two_from = two.seen.len();
        two.say(PROVE, "Proving preparation queued").await;
        retained_preparation(&mut two, two_from, "two").await;
        let two_preparation_ms = u64::try_from(two_preparing.elapsed().as_millis()).unwrap();
        println!(
            "M6_LIVE_PROVING_PREPARATION {}",
            json!({"account":"two", "elapsed_ms":two_preparation_ms})
        );
        one.say(PROVE, "An account already has an admitted bulk job.")
            .await;
        overflow
            .say(PROVE, "The bounded compute queue is full.")
            .await;
        let loaded_match = start_live(&server, &mut pilot).await;
        assert_ne!(loaded_match, baseline_match, "a fresh normal loaded sortie");
        let initial = books(&server).await;
        check_bounds(&initial);
        assert!(
            initial.iter().all(|book| book["active"] == 1
                && matches!(run(book)["status"].as_str(), Some("queued" | "running"))),
            "both private jobs remain active after loaded boarding: {initial:?}"
        );
        assert_eq!(
            initial.len(),
            2,
            "both accounts admitted on the same server"
        );
        assert_ne!(initial[0]["id"], initial[1]["id"]);
        assert_ne!(run(&initial[0])["character"], run(&initial[1])["character"]);
        // Anchor every measurement after fresh boarding and both admissions.
        // Neither baseline nor preparation progress can satisfy loaded progress.
        let loaded_before =
            require_live_time(&server, &mut pilot, loaded_match, "loaded start").await;
        let starts = initial
            .iter()
            .map(|book| {
                (
                    book["id"].clone(),
                    number(run(book), "next_sequence"),
                    settled_charge(book),
                )
            })
            .collect::<Vec<_>>();
        let mut loaded = Vec::new();
        let progressed;
        loop {
            loaded.push(status_sample(&mut pilot).await);
            let rows = books(&server).await;
            check_bounds(&rows);
            assert_eq!(rows.len(), 2);
            for book in &rows {
                assert_eq!(
                    book["active"], 1,
                    "both workloads remain admitted during live measurements"
                );
                assert!(matches!(
                    run(book)["status"].as_str(),
                    Some("queued" | "running")
                ));
            }
            let fair = starts.iter().all(|(id, first, charged)| {
                rows.iter().any(|book| {
                    book["id"] == *id
                        && number(run(book), "next_sequence") >= first + 4
                        && settled_charge(book) > *charged
                })
            });
            let loaded_now =
                require_live_time(&server, &mut pilot, loaded_match, "loaded measurement").await;
            if loaded.len() >= 16 && fair && loaded_now >= loaded_before + 2_000 {
                progressed = rows;
                break;
            }
            tokio::time::sleep(Duration::from_millis(75)).await;
        }
        let after = require_live_time(&server, &mut pilot, loaded_match, "loaded finish").await;
        assert!(
            after >= loaded_before + 2_000,
            "the real live arena advanced while both private runs advanced"
        );
        assert!(
            server
                .store
                .module_json_get("tacsim", &format!("tacsim:match:{loaded_match}"))
                .await
                .unwrap()
                .is_none(),
            "the live match did not end before the loaded observation"
        );
        // Timing is evidence, never an arbitrary performance pass/fail ratio.
        println!(
            "M6_LIVE_PROVING_ISOLATION {}",
            json!({"servers":1,"proving_workers":1,
            "baseline_match":baseline_match,"loaded_match":loaded_match,
            "baseline_result":baseline_result,"queued_limit":1,"daily_unit_minutes":DAILY_MINUTES,"live_before_ms":before,
            "live_loaded_before_ms":loaded_before,
            "preparation_ms":{"one":one_preparation_ms,"two":two_preparation_ms},
            "live_after_ms":after,"baseline":timings(&baseline),"loaded":timings(&loaded),
            "initial_accounts":initial,"progressed_accounts":progressed})
        );

        tokio::join!(
            one.say("@mind/cancel 1", "Cancellation recorded"),
            two.say("@mind/cancel 1", "Cancellation recorded")
        );
        loop {
            let rows = books(&server).await;
            check_bounds(&rows);
            if rows.iter().all(|book| {
                book["active"].is_null()
                    && number(book, "reserved_unit_ms") == 0
                    && run(book)["status"] == "cancelled"
            }) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        pilot.say("SIM CONCEDE", "concedes").await;
        pilot.close().await;
        one.close().await;
        two.close().await;
        overflow.close().await;
    })
    .await
    .expect("bounded admission, worker and live-progress liveness");
}

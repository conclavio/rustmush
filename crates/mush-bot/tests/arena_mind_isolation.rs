//! M6.6: two proving accounts share one production server with a live arena.
//! Public SSH commands cause every action. Durable reads assert resource/custody
//! facts only; no read is fed into a pilot or used to fabricate worker progress.
use serde_json::{Value, json};
use std::time::{Duration, Instant};

#[path = "support/arena.rs"]
mod rig;

const PROVE: &str = "@mind/prove Standard/Line VS Standard/Line 100 MUSTER 6v6 SEED 713";
const DAILY_MINUTES: u64 = 2_000;

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
        assert_eq!(row["plan"]["unit_count"], 12);
        assert_eq!(row["plan"]["games"], 100);
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
async fn live_time(server: &rig::Rig) -> Option<i64> {
    let value = server
        .store
        .module_json_get("tacsim", "tacsim:snapshot:1")
        .await
        .unwrap()?;
    let state: Value =
        serde_json::from_str(value["state"].as_str().expect("canonical snapshot")).unwrap();
    state["now_ms"].as_i64()
}
struct StatusSample {
    elapsed_us: u64,
    transient_retries: [usize; 4],
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
    tokio::time::timeout(Duration::from_secs(150), async {
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
        pilot.say("SIM TEAM red", "takes red slot 1").await;
        pilot.say("SIM START", "is boarding").await;
        pilot.say("red", "Red Room").await;
        pilot.say("BOARD", "climb into").await;
        pilot.expect("Good hunting", rig::STEP_TIMEOUT).await;
        let mut baseline = Vec::new();
        for _ in 0..16 {
            baseline.push(status_sample(&mut pilot).await);
            tokio::time::sleep(Duration::from_millis(75)).await;
        }
        assert!(
            books(&server).await.is_empty(),
            "baseline has no private work"
        );
        let before = live_time(&server)
            .await
            .expect("live arena snapshot before proving");

        one.say(PROVE, "Proving preparation queued").await;
        one.expect(
            "Proving run 1 retained and queued.",
            Duration::from_secs(40),
        )
        .await;
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
        two.say(PROVE, "Proving preparation queued").await;
        two.expect(
            "Proving run 1 retained and queued.",
            Duration::from_secs(40),
        )
        .await;
        one.say(PROVE, "An account already has an admitted bulk job.")
            .await;
        overflow
            .say(PROVE, "The bounded compute queue is full.")
            .await;
        let initial = books(&server).await;
        check_bounds(&initial);
        assert_eq!(
            initial.len(),
            2,
            "both accounts admitted on the same server"
        );
        assert_ne!(initial[0]["id"], initial[1]["id"]);
        assert_ne!(run(&initial[0])["character"], run(&initial[1])["character"]);
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
            if loaded.len() >= 16
                && fair
                && live_time(&server)
                    .await
                    .is_some_and(|now| now >= before + 2_000)
            {
                progressed = rows;
                break;
            }
            tokio::time::sleep(Duration::from_millis(75)).await;
        }
        let after = live_time(&server).await.unwrap();
        assert!(
            after > before,
            "the real live arena advanced while both private runs advanced"
        );
        assert!(
            server
                .store
                .module_json_get("tacsim", "tacsim:match:1")
                .await
                .unwrap()
                .is_none(),
            "the live match did not end before the loaded observation"
        );
        // Timing is evidence, never an arbitrary performance pass/fail ratio.
        println!(
            "M6_LIVE_PROVING_ISOLATION {}",
            json!({"servers":1,"proving_workers":1,
            "queued_limit":1,"daily_unit_minutes":DAILY_MINUTES,"live_before_ms":before,
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

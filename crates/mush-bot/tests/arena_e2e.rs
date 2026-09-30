//! **The bot flies a match** (cut AJ.2): the real `mush-bot` binary, `--arena
//! red --headless` with no model, against the production composition with the
//! simulator centre open, booted in this test binary by the AJ.1 rig.
//!
//! The pilot walks from Limbo to the centre, claims red, leads the side (it is
//! first on it), starts the match, boards, powers up and fights one AI-filled
//! machine to the match's end over its own SSH line session, then reports. Staff
//! re-pose both tanks 500 m apart on the east-west road before anyone claims
//! (the stock muster is 2 430 m apart against a 600 m sight); the override
//! survives the claim's reset to muster and is consumed by the match.
//!
//! **What is asserted is honesty, not a win** (16c Q-AJ1): one match on the
//! centre's own result word; the pilot fired and hit; `success` exactly when the
//! word is red, and the exit code with it; every line it typed is in its
//! vocabulary and never a forbidden verb; no second of its own clock held more
//! than four of them; nothing but the lane typed; and its memory's recent window
//! holds the match's end but no poll reply and no order acknowledgement.
//!
//! **The rest of the proof** (cut AJ.4), each against the same rig and with the
//! tanks posed inside each other's sight first:
//!
//! - **the wingman**: a human (`ada`, the rig's own line client) claims red
//!   first and so leads; the bot claims beside her, never starts the match nor
//!   types anything of a lead's, boards, powers up and fires, and reports the
//!   abort she calls as the non-win it is;
//! - **the commander on a stub**: a `wiremock` model answering a doctrine after
//!   300 ms is asked with one call in flight at a time, each request carrying
//!   exactly the table's `reasoning` and `provider`, the commander's own policy
//!   and a picture of at most 2 KB, each filed as `arena_commander`;
//! - **the goal action**: a plain `--goal` whose model answers `arena` once
//!   hands the pod to the pilot, which flies from the walk to `FIRE` with no
//!   commander and never asks that model anything again.
//!
//! No test here calls a model API: every model is a `wiremock` stub, and every
//! child runs from a directory holding an empty `.env` with every model key
//! removed ([`arena_rig::bot_command`]).
//!
//! Every arena test in this file holds one file-wide async mutex, so two arena
//! worlds never run at once in this binary.
use std::{
    sync::{Arc, LazyLock, Mutex},
    time::{Duration, Instant},
};

use mush_bot::arena::{Line, Side, classify, commander::COMMANDER_POLICY};
use serde_json::{Value, json};
use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::method};

#[path = "support/arena.rs"]
mod arena_rig;

/// One arena world at a time in this binary.
static ARENA: LazyLock<tokio::sync::Mutex<()>> = LazyLock::new(|| tokio::sync::Mutex::new(()));

/// The lines the pilot may type.
fn allowed(line: &str) -> bool {
    const EXACT: [&str; 17] = [
        "look",
        "ooc",
        "simulator",
        "red",
        "blue",
        "SIM STATUS",
        "SIM TEAM red",
        "SIM TEAM blue",
        "SIM START",
        "BOARD",
        "STARTUP",
        "SIM LEAVE",
        "CONTACTS",
        "WEAPONSTATUS",
        "STATUS",
        "RANGE",
        "BEARING B",
    ];
    if EXACT.contains(&line) {
        return true;
    }
    let number = |rest: &str| {
        let digits = rest.strip_prefix('-').unwrap_or(rest);
        !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
    };
    if let Some(rest) = line.strip_prefix("HEADING ") {
        return number(rest) && !rest.starts_with('-');
    }
    if let Some(rest) = line.strip_prefix("SPEED ") {
        return number(rest);
    }
    if let Some(rest) = line.strip_prefix("LOCK ") {
        return !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_alphanumeric());
    }
    if let Some(rest) = line.strip_prefix("FIRE ") {
        return !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_lowercase() || b == b'_');
    }
    false
}

/// The verbs the pilot must never type (16c Q-AJ2, Q-AJ3).
fn forbidden(line: &str) -> bool {
    let upper = line.to_ascii_uppercase();
    ["SIM AI", "SIM UNIT", "SIM MODE", "SIM RANGE", "ORDER"]
        .iter()
        .any(|verb| upper == *verb || upper.starts_with(&format!("{verb} ")))
        || ["SIM CONCEDE", "SIM ABORT", "EXIT"].contains(&upper.as_str())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_bot_flies_a_match_alone_against_one_machine() {
    let _one_world = ARENA.lock().await;
    tokio::time::timeout(Duration::from_secs(200), fly_alone())
        .await
        .expect("the solo flight finishes inside 200 s");
}

async fn fly_alone() {
    let mut cfg = arena_rig::arena_config();
    cfg.match_.startup_ms = 2_000;
    cfg.match_.board_timeout_ms = 60_000;
    cfg.match_.max_duration_ms = 60_000;
    let rig = arena_rig::boot(cfg, &[("pilot", "Pilot")]).await;

    // Staff pose both tanks 500 m apart on the road, on the idle centre, before
    // anyone claims.
    let mut god =
        arena_rig::Client::login(&rig, arena_rig::GOD_ACCOUNT, arena_rig::GOD_PASSWORD, "God")
            .await;
    arena_rig::pose(&mut god, "RA=100 97 90").await;
    arena_rig::pose(&mut god, "BA=150 97 270").await;

    let config = arena_rig::bot_config(&rig, "pilot+Pilot", "");
    let transcript = rig.dir.join("bot").join("solo.jsonl");
    let report_path = rig.dir.join("bot").join("solo.json");
    let mut command = tokio::process::Command::from(arena_rig::bot_command(
        &rig,
        &config,
        arena_rig::PILOT_PASSWORD,
    ));
    command
        .arg("--arena")
        .arg("red")
        .arg("--headless")
        .arg("--transcript")
        .arg(&transcript)
        .arg("--report")
        .arg(&report_path)
        .kill_on_drop(true);
    let started = std::time::Instant::now();
    let output = tokio::time::timeout(Duration::from_secs(150), command.output())
        .await
        .expect("the bot finishes inside 150 s")
        .expect("the bot runs");
    let wall = started.elapsed();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let report_text = std::fs::read_to_string(&report_path).unwrap_or_else(|error| {
        panic!("the report ({error}); stdout:\n{stdout}\nstderr:\n{stderr}")
    });
    let report: Value = serde_json::from_str(&report_text).expect("the report is JSON");
    let transcript_text = std::fs::read_to_string(&transcript).expect("the transcript");
    let records: Vec<Value> = transcript_text
        .lines()
        .map(|line| serde_json::from_str(line).expect("a transcript record"))
        .collect();

    // One match, on the centre's own word.
    let matches = report["arena"]["matches"]
        .as_array()
        .unwrap_or_else(|| panic!("the arena block; stdout:\n{stdout}\nstderr:\n{stderr}"));
    assert_eq!(matches.len(), 1, "one match: {report_text}");
    let flown = &matches[0];
    let result = flown["result"].as_str().expect("a result word");
    let shots = flown["shots"].as_u64().expect("shots");
    let hits = flown["hits_scored"].as_u64().expect("hits");
    println!(
        "SOLO FLIGHT: result {result}, {shots} shot(s), {hits} hit(s), wall {:.1} s; outcome: {}",
        wall.as_secs_f64(),
        report["state"]["outcome"]
    );
    assert!(
        ["red", "blue", "draw", "timeout"].contains(&result),
        "a fought result, not {result}: {report_text}"
    );
    assert!(shots >= 1, "the pilot fired: {report_text}");
    assert!(hits >= 1, "the pilot hit: {report_text}");

    // Success exactly on the pilot's own side's word, and the exit code with it.
    let success = report["success"].as_bool().expect("success");
    assert_eq!(success, result == "red");
    assert_eq!(
        output.status.code(),
        Some(if success { 0 } else { 2 }),
        "stderr:\n{stderr}"
    );

    // Every line typed: in the vocabulary, never a forbidden verb, at most four
    // in any second of the lane's own clock.
    let sends: Vec<(String, u64)> = records
        .iter()
        .filter(|record| record["kind"] == "arena-send")
        .map(|record| {
            (
                record["data"]["line"].as_str().expect("a line").to_owned(),
                record["data"]["lane_ms"].as_u64().expect("a lane stamp"),
            )
        })
        .collect();
    assert!(sends.len() > 20, "the pilot typed: {sends:?}");
    for (line, _) in &sends {
        assert!(allowed(line), "`{line}` is not the pilot's to type");
        assert!(!forbidden(line), "`{line}` is forbidden");
    }
    for window in sends.windows(5) {
        assert!(
            window[4].1 >= window[0].1 + 1_000,
            "five lines inside one second: {window:?}"
        );
    }
    let lines: Vec<&str> = sends.iter().map(|(line, _)| line.as_str()).collect();
    for expected in ["look", "SIM TEAM red", "SIM START", "BOARD", "STARTUP"] {
        assert!(lines.contains(&expected), "{expected} in {lines:?}");
    }
    assert!(lines.iter().any(|line| line.starts_with("FIRE ")));

    // Nothing but the lane typed: every command the run counts is an
    // `arena-send`, and every `send` event is one of its walk or lobby lines
    // (the cockpit's are recorded, not echoed).
    assert_eq!(
        report["state"]["commands_sent"].as_u64(),
        u64::try_from(sends.len()).ok(),
        "a line went out that was not the lane's"
    );
    const LOBBY: [&str; 12] = [
        "look",
        "ooc",
        "simulator",
        "red",
        "blue",
        "SIM STATUS",
        "SIM TEAM red",
        "SIM TEAM blue",
        "SIM START",
        "BOARD",
        "STARTUP",
        "SIM LEAVE",
    ];
    let echoed: Vec<&str> = records
        .iter()
        .filter(|record| record["kind"] == "send")
        .map(|record| record["data"].as_str().expect("a sent line"))
        .collect();
    assert!(!echoed.is_empty());
    for line in &echoed {
        assert!(LOBBY.contains(line), "`{line}` was sent outside the lane");
    }

    // The recent window is the journal: it holds what happened (the match's
    // end), and no poll reply or order acknowledgement.
    let recent = report["state"]["recent"].as_str().expect("recent");
    assert!(
        recent.contains(" is over ("),
        "the journal reached the recent window: {recent}"
    );
    for line in recent.lines() {
        assert!(
            !matches!(
                classify(line),
                Line::ContactRow(_)
                    | Line::NoContacts
                    | Line::WeaponRow(_)
                    | Line::StatusHead(_)
                    | Line::Steering { .. }
                    | Line::Making { .. }
                    | Line::Firing { .. }
            ),
            "a read reply or an ack in the recent window: {line}"
        );
    }
    god.close().await;
    drop(rig);
}

// ---------------------------------------------------------------------------
// Cut AJ.4: the wingman, the commander and the goal action
// ---------------------------------------------------------------------------

/// The lines a wingman must never type: the forbidden set, and `SIM START`,
/// which is the lead's (16c Q-AJ3).
fn not_a_wingmans(line: &str) -> bool {
    forbidden(line) || line.eq_ignore_ascii_case("SIM START")
}

/// **The bot flies beside a human lead** (16c Q-AJ3). God poses all four tanks
/// of a two-a-side fill about 500 m apart on and beside the road; `ada` claims
/// red first and so leads; the bot (`--arena red`, no model, no lead's grace)
/// claims second. Once `SIM STATUS` shows the two of them she starts, boards,
/// and calls it off once the bot has fired (40 s at most). The bot never starts
/// it nor types anything of a lead's, boards, powers up and fires, and reports
/// the abort as a non-win.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_bot_flies_as_a_wingman_beside_a_human() {
    let _one_world = ARENA.lock().await;
    tokio::time::timeout(Duration::from_secs(150), fly_as_wingman())
        .await
        .expect("the wingman flight finishes inside 150 s");
}

async fn fly_as_wingman() {
    let mut cfg = arena_rig::arena_config();
    cfg.match_.startup_ms = 2_000;
    cfg.match_.board_timeout_ms = 60_000;
    cfg.match_.max_duration_ms = 100_000;
    let rig = arena_rig::boot(cfg, &[("pilot", "Pilot"), ("ada", "Ada")]).await;

    // Every tank of the fill about 500 m from the other side, on the idle
    // centre, before anyone claims.
    let mut god =
        arena_rig::Client::login(&rig, arena_rig::GOD_ACCOUNT, arena_rig::GOD_PASSWORD, "God")
            .await;
    for spec in [
        "RA=100 97 90",
        "RB=100 95 90",
        "BA=150 97 270",
        "BB=150 95 270",
    ] {
        arena_rig::pose(&mut god, spec).await;
    }

    // The human is first on red, so she leads.
    let mut ada = arena_rig::Client::login(&rig, "ada", arena_rig::PILOT_PASSWORD, "Ada").await;
    arena_rig::walk_to_lobby(&mut ada).await;
    ada.say("SIM TEAM red", "takes red slot 1").await;

    // No lead's grace: a bot that wrongly took itself for the lead would type
    // `SIM START` at once on its claim, before she reads the board and starts,
    // so the check below could see it (with the default 3 s her start would
    // always come first and hide it).
    let config = arena_rig::bot_config(&rig, "pilot+Pilot", "\n[arena]\nlead_start_grace_ms = 0\n");
    let mut flight = arena_rig::launch(&rig, &config, "wingman", &["--arena", "red"]);

    // Until her board shows two red pilots (the bot's claim).
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let (found, text) = ada.answer("SIM STATUS", &["AI fill:"]).await;
        assert!(found.is_some(), "the board answers: {text}");
        let reds = text
            .lines()
            .find_map(|line| match classify(line) {
                Line::BoardSide {
                    side: Side::Red,
                    pilots,
                } => Some(pilots.len()),
                _ => None,
            })
            .unwrap_or(0);
        if reds >= 2 {
            break;
        }
        assert!(
            !flight.task.is_finished(),
            "the bot ended before it claimed: {text}"
        );
        assert!(
            Instant::now() < deadline,
            "the bot never claimed beside her: {text}"
        );
        ada.idle(Duration::from_secs(1)).await;
    }

    // She starts and boards; once the bot has fired (at most 40 s on), she
    // calls it off, well before two machines could end it on their own.
    ada.say("SIM START", "is boarding").await;
    ada.say("red", "Red Room").await;
    ada.say("BOARD", "You climb into").await;
    let from = ada.seen.len();
    let hunting = arena_rig::await_line(
        &mut ada,
        from,
        "Good hunting",
        &mut flight,
        Duration::from_secs(60),
    )
    .await;
    arena_rig::await_fire(&mut ada, &mut flight, Duration::from_secs(40)).await;
    arena_rig::still_running(&ada, hunting);
    ada.say("SIM ABORT", "aborts match").await;

    let landed = arena_rig::land(flight, Duration::from_secs(30)).await;
    let sends = landed.sends();
    let lines: Vec<&str> = sends.iter().map(|(line, _, _)| line.as_str()).collect();
    for line in &lines {
        assert!(allowed(line), "`{line}` is not the pilot's to type");
        assert!(!not_a_wingmans(line), "the wingman typed `{line}`");
    }
    for expected in ["SIM TEAM red", "BOARD", "STARTUP"] {
        assert!(lines.contains(&expected), "{expected} in {lines:?}");
    }
    assert!(
        lines.iter().any(|line| line.starts_with("FIRE ")),
        "the wingman fired: {lines:?}"
    );

    let matches = landed.matches();
    assert_eq!(matches.len(), 1, "one match: {}", landed.report_text);
    let flown = &matches[0];
    assert_eq!(flown["lead"], false, "{}", landed.report_text);
    assert_eq!(flown["callsign"], "RB", "the second red slot");
    assert_eq!(flown["result"], "abort");
    assert_eq!(flown["won"], false);
    assert_eq!(landed.report["success"], false);
    assert_eq!(landed.code, Some(2), "stderr:\n{}", landed.stderr);
    println!(
        "WINGMAN: {} shot(s), {} hit(s) as {} beside Ada",
        flown["shots"], flown["hits_scored"], flown["callsign"]
    );
    ada.close().await;
    god.close().await;
    drop(rig);
}

/// The stub commander's one answer: a valid doctrine, OpenRouter-shaped, priced.
/// Its band and sense differ from the pilot's defaults (450 m, `auto`), so the
/// next picture shows whether the lane adopted it.
const STUB_DOCTRINE: &str =
    r#"{"target":null,"stance":"orbit","band_m":300,"sense":"cw","note":"tighten to 300 m"}"#;
/// How long the stub commander takes to answer.
const STUB_DELAY: Duration = Duration::from_millis(300);

/// **The commander flies on a stub** (cut AJ.3's commander, end to end). The
/// bot (`--arena red`, alone against one machine) carries `[arena.commander_model]`
/// pointing at a `wiremock` stub that answers every call with a valid doctrine
/// (a 300 m band, clockwise) after 300 ms; God calls the match off once three
/// calls have been answered (or 30 s after `Good hunting`). One call is in
/// flight at a time, each request carries exactly the table's routing, the
/// commander's policy and a picture of at most 2 KB, every picture after the
/// first carries the stub's doctrine as the one the lane flies, and each call
/// is filed under `arena_commander`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_commander_sets_doctrine_on_a_stub() {
    let _one_world = ARENA.lock().await;
    tokio::time::timeout(Duration::from_secs(120), command_on_a_stub())
        .await
        .expect("the commanded flight finishes inside 120 s");
}

async fn command_on_a_stub() {
    // Each request's body and arrival; its response goes out STUB_DELAY later.
    let arrivals: Arc<Mutex<Vec<(Instant, Value)>>> = Arc::default();
    let stub = MockServer::start().await;
    let heard = Arc::clone(&arrivals);
    Mock::given(method("POST"))
        .respond_with(move |request: &Request| {
            let body: Value = request.body_json().unwrap_or(Value::Null);
            heard
                .lock()
                .expect("the arrivals")
                .push((Instant::now(), body));
            ResponseTemplate::new(200)
                .set_delay(STUB_DELAY)
                .set_body_json(json!({
                    "id": "gen-commander", "model": "deepseek/deepseek-v4.1-flash",
                    "provider": "Together",
                    "choices": [{"message": {"content": STUB_DOCTRINE}, "finish_reason": "stop"}],
                    "usage": {"cost": 0.00015, "prompt_tokens": 400, "completion_tokens": 30,
                        "total_tokens": 430}
                }))
        })
        .mount(&stub)
        .await;

    let mut cfg = arena_rig::arena_config();
    cfg.match_.startup_ms = 2_000;
    cfg.match_.board_timeout_ms = 60_000;
    cfg.match_.max_duration_ms = 60_000;
    let rig = arena_rig::boot(cfg, &[("pilot", "Pilot")]).await;
    let mut god =
        arena_rig::Client::login(&rig, arena_rig::GOD_ACCOUNT, arena_rig::GOD_PASSWORD, "God")
            .await;
    arena_rig::pose(&mut god, "RA=100 97 90").await;
    arena_rig::pose(&mut god, "BA=150 97 270").await;
    arena_rig::walk_to_lobby(&mut god).await;

    let config = arena_rig::bot_config(
        &rig,
        "pilot+Pilot",
        &format!(
            r#"
[arena]
commander = true
commander_ms = 1000

[arena.commander_model]
endpoint = "{}/v1/chat/completions"
name = "deepseek/deepseek-v4.1-flash"
max_tokens = 200
cost_currency = "USD"
reasoning = {{ enabled = false }}
provider = {{ order = ["Together"], allow_fallbacks = false }}
"#,
            stub.uri()
        ),
    );
    let from = god.seen.len();
    let mut flight = arena_rig::launch(&rig, &config, "commander", &["--arena", "red"]);
    let hunting = arena_rig::await_line(
        &mut god,
        from,
        "Good hunting",
        &mut flight,
        Duration::from_secs(60),
    )
    .await;

    // Three answered calls, or 30 s, whichever is first; then the abort.
    let deadline = Instant::now() + Duration::from_secs(30);
    while arrivals.lock().expect("the arrivals").len() < 3 && Instant::now() < deadline {
        assert!(!flight.task.is_finished(), "the bot ended mid-match");
        god.idle(Duration::from_millis(100)).await;
    }
    god.idle(STUB_DELAY + Duration::from_millis(100)).await;
    arena_rig::still_running(&god, hunting);
    god.say("@arena/abort", "is stopping").await;

    let landed = arena_rig::land(flight, Duration::from_secs(30)).await;
    let arrivals = arrivals.lock().expect("the arrivals").clone();
    let asked = arrivals.len();
    assert!(asked >= 3, "at least three calls reached the stub: {asked}");

    // One call in flight at a time: each arrives after the previous answer
    // left the stub.
    for pair in arrivals.windows(2) {
        assert!(
            pair[1].0 >= pair[0].0 + STUB_DELAY,
            "a call arrived {} ms after the previous one, before its answer left",
            pair[1].0.duration_since(pair[0].0).as_millis()
        );
    }

    // Every request: exactly the table's routing, unstreamed, the commander's
    // own policy, and a picture of at most 2 KB.
    for (_, body) in &arrivals {
        let keys: std::collections::BTreeSet<&str> = body
            .as_object()
            .expect("a JSON body")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            std::collections::BTreeSet::from([
                "max_tokens",
                "messages",
                "model",
                "provider",
                "reasoning",
                "stream"
            ]),
            "{body}"
        );
        assert_eq!(body["reasoning"], json!({"enabled": false}));
        assert_eq!(
            body["provider"],
            json!({"order": ["Together"], "allow_fallbacks": false})
        );
        assert_eq!(body["stream"], false);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][0]["content"], COMMANDER_POLICY);
        let picture = body["messages"][1]["content"]
            .as_str()
            .expect("the picture as text");
        assert!(
            picture.len() <= 2_048,
            "a picture of {} bytes",
            picture.len()
        );
        let picture: Value = serde_json::from_str(picture).expect("the picture is JSON");
        assert!(picture.is_object(), "{picture}");
    }

    // The lane flies what it was told: the first picture carries the pilot's
    // own doctrine, and every later one (asked only after the previous answer
    // came back) the stub's band and sense. The `note` is left out, being the
    // first thing a picture over 2 KB drops.
    let doctrine = |body: &Value| -> Value {
        let picture: Value = serde_json::from_str(
            body["messages"][1]["content"]
                .as_str()
                .expect("the picture as text"),
        )
        .expect("the picture is JSON");
        json!([picture["doctrine"]["band_m"], picture["doctrine"]["sense"]])
    };
    assert_eq!(
        doctrine(&arrivals[0].1),
        json!([450, "auto"]),
        "the first ask flies the pilot's defaults"
    );
    for (index, (_, body)) in arrivals.iter().enumerate().skip(1) {
        assert_eq!(
            doctrine(body),
            json!([300, "cw"]),
            "ask {} flies the stub's doctrine",
            index + 1
        );
    }

    // The transcript: the commander named at the lane's start, and a doctrine
    // applied.
    let said = landed.events("arena");
    assert!(
        said.iter()
            .any(|line| line
                == "Commander: deepseek/deepseek-v4.1-flash from [arena.commander_model]."),
        "the commander's start line: {said:?}"
    );
    assert!(
        landed
            .records
            .iter()
            .any(|record| record["kind"] == "arena-doctrine" && record["data"]["band_m"] == 300),
        "the stub's doctrine applied"
    );

    // The cost ledger: every call the stub heard is an `arena_commander`
    // request; one the bot cancelled at the end may not have reached it.
    let filed = &landed.report["costs"]["session"]["by_prompt_type"]["arena_commander"];
    let requests = filed["requests"].as_u64().expect("requests");
    let cancelled = filed["cancelled"].as_u64().unwrap_or(0);
    let heard = u64::try_from(asked).expect("a count");
    assert!(
        requests >= heard && requests <= heard + cancelled,
        "{requests} filed ({cancelled} cancelled) against {heard} heard: {filed}"
    );
    assert!(
        landed.report["costs"]["session"]["by_prompt_type"]["goal_decision"].is_null(),
        "no goal turn was asked"
    );

    let matches = landed.matches();
    assert_eq!(matches.len(), 1, "{}", landed.report_text);
    let flown = &matches[0];
    assert!(
        flown["commander_calls"].as_u64().expect("calls") >= 3,
        "{}",
        landed.report_text
    );
    // Every answer but perhaps the last (cancelled by the abort) was flown.
    let applied = flown["commander_applied"].as_u64().expect("applied");
    assert!(
        applied + 1 >= heard && applied <= flown["commander_calls"].as_u64().expect("calls"),
        "{applied} applied of {heard} heard: {}",
        landed.report_text
    );
    assert_eq!(flown["commander_stale"], 0, "a 300 ms answer is never late");
    assert_eq!(flown["result"], "abort");
    assert_eq!(landed.report["success"], false);
    assert_eq!(landed.code, Some(2), "stderr:\n{}", landed.stderr);
    println!(
        "COMMANDER: {asked} call(s) heard, {} applied, {requests} filed",
        flown["commander_applied"]
    );
    god.close().await;
    drop(rig);
}

/// **A plain goal hands the pod to the pilot** (cut AJ.3's `arena` action, end
/// to end). The goal model (a `wiremock` stub) answers its first request with
/// `arena` for red; the pilot then flies from the walk to `FIRE` alone against
/// one machine with no commander (no `[arena.commander_model]`, and the
/// session's model commands only by opt-in), until God calls the match off
/// once it has fired.
/// The goal model is asked exactly once.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_plain_goal_hands_the_pod_to_the_pilot() {
    let _one_world = ARENA.lock().await;
    tokio::time::timeout(Duration::from_secs(120), hand_the_pod_over())
        .await
        .expect("the goal's flight finishes inside 120 s");
}

async fn hand_the_pod_over() {
    let asked = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let stub = MockServer::start().await;
    let count = Arc::clone(&asked);
    Mock::given(method("POST"))
        .respond_with(move |_: &Request| {
            // Any second request of any kind is a failure (asserted below):
            // the goal model must never be asked again, least of all as a
            // commander. It is refused here so nothing it said could steer.
            if count.fetch_add(1, std::sync::atomic::Ordering::SeqCst) > 0 {
                return ResponseTemplate::new(500).set_body_string("asked twice");
            }
            ResponseTemplate::new(200).set_body_json(json!({
                "id": "gen-goal", "model": "fixture-goal",
                "choices": [{"message": {"content": json!({
                    "action": "arena", "side": "red",
                    "summary": "Handing the pod to the pilot."
                }).to_string()}, "finish_reason": "stop"}],
                "usage": {"cost": 0.001, "prompt_tokens": 900, "completion_tokens": 20,
                    "total_tokens": 920}
            }))
        })
        .mount(&stub)
        .await;

    let mut cfg = arena_rig::arena_config();
    cfg.match_.startup_ms = 2_000;
    cfg.match_.board_timeout_ms = 60_000;
    cfg.match_.max_duration_ms = 60_000;
    let rig = arena_rig::boot(cfg, &[("pilot", "Pilot")]).await;
    let mut god =
        arena_rig::Client::login(&rig, arena_rig::GOD_ACCOUNT, arena_rig::GOD_PASSWORD, "God")
            .await;
    arena_rig::pose(&mut god, "RA=100 97 90").await;
    arena_rig::pose(&mut god, "BA=150 97 270").await;
    arena_rig::walk_to_lobby(&mut god).await;

    let config = arena_rig::bot_config(
        &rig,
        "pilot+Pilot",
        &format!(
            r#"
[model]
endpoint = "{}/v1/chat/completions"
name = "fixture-goal"
cost_currency = "USD"
"#,
            stub.uri()
        ),
    );
    let from = god.seen.len();
    let mut flight = arena_rig::launch(
        &rig,
        &config,
        "goal",
        &["--goal", "Beat the blue team in the simulator"],
    );
    let hunting = arena_rig::await_line(
        &mut god,
        from,
        "Good hunting",
        &mut flight,
        Duration::from_secs(90),
    )
    .await;
    // Called off once the pilot has fired (30 s at most): the posed machine
    // has ended a 1 v 1 in about thirty seconds on its own.
    arena_rig::await_fire(&mut god, &mut flight, Duration::from_secs(30)).await;
    arena_rig::still_running(&god, hunting);
    god.say("@arena/abort", "is stopping").await;

    let landed = arena_rig::land(flight, Duration::from_secs(30)).await;
    assert_eq!(
        asked.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the goal model was asked exactly once"
    );
    assert_eq!(
        landed.report["state"]["model_calls"], 1,
        "{}",
        landed.report_text
    );
    assert!(
        landed
            .events("arena")
            .iter()
            .any(|line| line == "No commander; the pilot flies on its defaults."),
        "no commander: {:?}",
        landed.events("arena")
    );

    // The lane typed from the walk to `FIRE`.
    let sends = landed.sends();
    let (first, purpose, _) = sends.first().expect("the lane typed");
    assert_eq!(
        (first.as_str(), purpose.as_str()),
        ("look", "walk"),
        "{sends:?}"
    );
    for (line, _, _) in &sends {
        assert!(allowed(line), "`{line}` is not the pilot's to type");
        assert!(!forbidden(line), "`{line}` is forbidden");
    }
    assert!(
        sends.iter().any(|(line, _, _)| line.starts_with("FIRE ")),
        "the pilot fired: {sends:?}"
    );

    let matches = landed.matches();
    assert_eq!(matches.len(), 1, "{}", landed.report_text);
    assert_eq!(matches[0]["result"], "abort");
    assert_eq!(landed.report["success"], false);
    assert_eq!(landed.code, Some(2), "stderr:\n{}", landed.stderr);
    god.close().await;
    drop(rig);
}

// ---------------------------------------------------------------------------
// The command line: every refusal comes before any connection, so these need
// no arena world (and take no lock)
// ---------------------------------------------------------------------------

/// Run the binary through the rig's own command builder in a scratch
/// directory; its exit code, stdout and stderr.
fn cli(args: &[&str]) -> (Option<i32>, String, String) {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "mush-bot-arena-cli-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    let config = dir.join("bot.toml");
    std::fs::write(
        &config,
        "[ssh]\nhost = \"127.0.0.1\"\nport = 1\nusername = \"pilot+Pilot\"\n\
         password_env = \"MUSH_BOT_ARENA_PASSWORD\"\n\
         [memory]\npersistent = false\n[costs]\npersistent = false\n",
    )
    .expect("a config");
    std::fs::write(dir.join("script.json"), r#"{"name":"x","steps":[]}"#).expect("a script");
    let output = arena_rig::bot_command_in(&dir, &config, "unused")
        .args(args)
        .output()
        .expect("the binary runs");
    let _ = std::fs::remove_dir_all(&dir);
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn the_command_line_refuses_what_the_pilot_cannot_fly() {
    let session = "--arena cannot be combined with --script, --paused, --managed-floor, --replay, \
                   --studio, --harness or a campaign session";
    for (args, sentence) in [
        (&["--arena", "green"][..], "--arena takes red or blue"),
        (&["--arena"][..], "--arena takes red or blue"),
        (
            &["--arena", "red", "--matches", "0"][..],
            "--matches takes a number from 1 to 100",
        ),
        (
            &["--arena", "red", "--matches", "101"][..],
            "--matches takes a number from 1 to 100",
        ),
        (
            &["--arena", "red", "--matches", "two"][..],
            "--matches takes a number from 1 to 100",
        ),
        (
            &["--matches", "2"][..],
            "--matches requires --arena or --goal",
        ),
        (
            &["--arena", "red", "--goal", "win"][..],
            "--arena flies now and --goal lets the model decide; choose one",
        ),
        (&["--arena", "red", "--script", "script.json"][..], session),
        (&["--arena", "blue", "--paused"][..], session),
        (&["--arena", "blue", "--managed-floor"][..], session),
        (
            &["--headless"][..],
            "headless play requires a goal and [model], --arena, or --script",
        ),
    ] {
        let (code, _, stderr) = cli(args);
        assert_eq!(code, Some(1), "{args:?}: {stderr}");
        assert_eq!(stderr.trim(), format!("mush-bot: {sentence}"), "{args:?}");
    }
    let (code, usage, _) = cli(&["--help"]);
    assert_eq!(code, Some(0));
    assert!(
        usage.contains("[--arena <red|blue> [--matches <n>]]"),
        "{usage}"
    );
}

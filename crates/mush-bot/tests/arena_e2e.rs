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
//! And (cut AK.3) **a refused pose answers at once**: `try_pose` comes back
//! with the verb's refusal sentence, not after the whole step timeout.
//!
//! And (cut AM.2) **the prompt pauses and resumes a flight**: at the bot's
//! own prompt, `/send STATUS` pauses the pilot (nothing of the lane's is typed
//! until `/resume`), and after `/resume` the pilot types again at once while
//! that command's output is still being collected.
//!
//! And (cut AM.3) **the commander runs through the runner**: over two
//! matches on a session budget of three requests, the third call (held by
//! the stub) is cancelled by the abort or the lane's own stale bound, the
//! budget blocks once at the first ask after three admissions and switches
//! the commander off for the run, every model record carries its own
//! request's id, no goal call is counted, and the pilot flies on.
//!
//! And (cut AN.2) **the opening reads the ground at the muster**: from the
//! shipped muster, with no pose, the pilot types `VIEW BRIEF` once, steps off
//! the road north or south by its own brief and holds there; the format
//! contract of the brief's bearing lines against the game's own.
//!
//! No test here calls a model API: every model is a `wiremock` stub, and every
//! child runs from a directory holding an empty `.env` with every model key
//! removed ([`arena_rig::bot_command`]).
//!
//! Every arena test in this file holds one file-wide async mutex, so two arena
//! worlds never run at once in this binary.
use std::{
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use mush_bot::arena::{
    Line, Side, classify,
    commander::{COMMANDER_POLICY, OFF_BUDGET},
};
use serde_json::{Value, json};
use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::method};

#[path = "support/arena.rs"]
mod arena_rig;

/// One arena world at a time in this binary.
static ARENA: LazyLock<tokio::sync::Mutex<()>> = LazyLock::new(|| tokio::sync::Mutex::new(()));

/// The lines the pilot may type.
fn allowed(line: &str) -> bool {
    const EXACT: [&str; 18] = [
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
        "VIEW BRIEF",
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
    // The mark before `BOARD` (cut AM.3): a `Good hunting` that comes in the
    // same chunk as her own climb-in is after it.
    let from = ada.seen.len();
    ada.say("BOARD", "You climb into").await;
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

/// How long the stub commander holds its third request: past the lane's stale
/// bound and the match, so only the bot can end that call.
const STUB_HOLD: Duration = Duration::from_secs(60);

/// The model records of a run, each filed under its own request's envelope.
const MODEL_RECORDS: [&str; 6] = [
    "model-input",
    "model-request",
    "model-reply",
    "usage",
    "cost-receipt",
    "model-cancelled",
];

/// **Each record's match**: the `match` of the last `arena-state` record at
/// or before it that names one.
fn match_of(records: &[Value]) -> Vec<Option<u64>> {
    let mut current = None;
    records
        .iter()
        .map(|record| {
            if record["kind"] == "arena-state"
                && let Some(id) = record["data"]["match"].as_u64()
            {
                current = Some(id);
            }
            current
        })
        .collect()
}

/// The matches the records name, in the order they first appear.
fn matches_named(records: &[Value]) -> Vec<u64> {
    let mut named: Vec<u64> = Vec::new();
    for id in match_of(records).into_iter().flatten() {
        if !named.contains(&id) {
            named.push(id);
        }
    }
    named
}

/// Where the run's one `budget-blocked` record is, and the `FIRE` typed in
/// the second match after it, once both are written.
fn blocked_then_fired(records: &[Value]) -> Option<(usize, usize)> {
    let blocked = records
        .iter()
        .position(|record| record["kind"] == "budget-blocked")?;
    let second = *matches_named(records).get(1)?;
    let of = match_of(records);
    let fired = records
        .iter()
        .enumerate()
        .skip(blocked)
        .find(|(index, record)| {
            record["kind"] == "arena-send"
                && record["data"]["line"]
                    .as_str()
                    .is_some_and(|line| line.starts_with("FIRE "))
                && of[*index] == Some(second)
        })
        .map(|(index, _)| index)?;
    Some((blocked, fired))
}

/// **The commander runs through the runner** (cut AM.3; wall clock,
/// generously bounded at 240 s). The bot flies `--arena red --matches 2`
/// alone against one machine posed 500 m off, with `[arena] commander` on a
/// `wiremock` stub and a session budget of `max_requests` 3 (no currency
/// limit) named by the config's top-level `budget`. The stub answers its
/// first two requests with a priced doctrine after 300 ms and **holds its
/// third 60 s**; God calls match 1 off as soon as the stub has heard that
/// third request, poses both tanks again (the lead's 5 s grace keeps the
/// bot's `SIM START` behind the poses), and calls match 2 off once the bot has
/// typed a `FIRE` after the budget's block.
///
/// The third call is cancelled by whichever comes first, the match's end or
/// the lane's own 2 s stale bound (`commander-stale` with `"abandoned":
/// true`); the test accepts either and prints which. The ledger counts every
/// request it admitted, a cancelled one included, and the budget blocks at
/// `requests >= max_requests`, so the block falls at the first ask after
/// three admissions — match 2's first when the abort won, match 1's next when
/// the stale bound did — and holds for the run: exactly one `budget-blocked`
/// record, the commander off with the budget's reason, and no fourth request.
/// Every model record carries the id its own `model-request` names, and each
/// request has its own records under it (the answered two their reply, usage
/// and receipt; the held third its cancel and no reply); the matches count
/// three commander calls, so the ask the budget refused was never counted
/// (the budget is checked before the ask is taken); the ledger files the calls
/// as `arena_commander` and none as a goal decision; no goal call is counted;
/// and both matches are reported as `abort`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_commander_runs_through_the_runner() {
    let _one_world = ARENA.lock().await;
    tokio::time::timeout(Duration::from_secs(240), command_through_the_runner())
        .await
        .expect("the two commanded matches finish inside 240 s");
}

async fn command_through_the_runner() {
    let started = Instant::now();
    // Each request's body and arrival, and how many there were; the third is
    // held, every other answered after STUB_DELAY.
    let arrivals: Arc<Mutex<Vec<(Instant, Value)>>> = Arc::default();
    let heard = Arc::new(AtomicUsize::new(0));
    let stub = MockServer::start().await;
    let (kept, count) = (Arc::clone(&arrivals), Arc::clone(&heard));
    Mock::given(method("POST"))
        .respond_with(move |request: &Request| {
            let body: Value = request.body_json().unwrap_or(Value::Null);
            kept.lock()
                .expect("the arrivals")
                .push((Instant::now(), body));
            let n = count.fetch_add(1, Ordering::SeqCst) + 1;
            ResponseTemplate::new(200)
                .set_delay(if n == 3 { STUB_HOLD } else { STUB_DELAY })
                .set_body_json(json!({
                    "id": format!("gen-commander-{n}"), "model": "deepseek/deepseek-v4.1-flash",
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
    cfg.match_.max_duration_ms = 100_000;
    let rig = arena_rig::boot(cfg, &[("pilot", "Pilot")]).await;
    let mut god =
        arena_rig::Client::login(&rig, arena_rig::GOD_ACCOUNT, arena_rig::GOD_PASSWORD, "God")
            .await;
    for spec in arena_rig::POSES {
        arena_rig::pose(&mut god, spec).await;
    }
    arena_rig::walk_to_lobby(&mut god).await;

    let config = arena_rig::bot_config(
        &rig,
        "pilot+Pilot",
        &format!(
            r#"
[arena]
commander = true
commander_ms = 1000
lead_start_grace_ms = 5000

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
    // The session budget beside the config, named by its top-level `budget`
    // (a top-level key must come before the first table).
    std::fs::write(
        rig.dir.join("bot").join("budget.json"),
        r#"{"version": 1, "max_requests": 3, "max_cost_by_currency": {}, "unknown_cost": "block"}"#,
    )
    .expect("the budget");
    let text = std::fs::read_to_string(&config).expect("the config");
    std::fs::write(&config, format!("budget = \"budget.json\"\n{text}"))
        .expect("the config names the budget");

    let from = god.seen.len();
    let mut flight = arena_rig::launch(
        &rig,
        &config,
        "runner",
        &["--arena", "red", "--matches", "2"],
    );
    let hunting = arena_rig::await_line(
        &mut god,
        from,
        "Good hunting",
        &mut flight,
        Duration::from_secs(60),
    )
    .await;

    // Match 1: called off as soon as the stub has heard its third request.
    let deadline = Instant::now() + Duration::from_secs(30);
    while heard.load(Ordering::SeqCst) < 3 {
        assert!(
            Instant::now() < deadline,
            "the stub heard {} request(s) in 30 s",
            heard.load(Ordering::SeqCst)
        );
        assert!(!flight.task.is_finished(), "the bot ended mid-match");
        god.idle(Duration::from_millis(20)).await;
    }
    arena_rig::still_running(&god, hunting);
    god.say("@arena/abort", "is stopping").await;
    let over = arena_rig::await_line(
        &mut god,
        hunting,
        " is over (",
        &mut flight,
        Duration::from_secs(30),
    )
    .await;
    // The end cleared the poses: both tanks again, for match 2.
    for spec in arena_rig::POSES {
        arena_rig::pose(&mut god, spec).await;
    }

    // Until the budget's block and a `FIRE` of match 2 after it (90 s at
    // most); the stub's count then.
    let deadline = Instant::now() + Duration::from_secs(90);
    let at_block = loop {
        if blocked_then_fired(&records_now(&flight.transcript)).is_some() {
            break heard.load(Ordering::SeqCst);
        }
        assert!(
            Instant::now() < deadline,
            "no budget block and FIRE in match 2 inside 90 s"
        );
        assert!(
            !flight.task.is_finished(),
            "the bot ended before match 2's FIRE"
        );
        god.idle(Duration::from_millis(100)).await;
    };
    arena_rig::still_running(&god, over);
    god.say("@arena/abort", "is stopping").await;

    let landed = arena_rig::land(flight, Duration::from_secs(60)).await;
    let wall = started.elapsed();
    let records = &landed.records;
    let total = heard.load(Ordering::SeqCst);

    // The block: once, the commander off for the budget, and no request after.
    let blocked: Vec<usize> = records
        .iter()
        .enumerate()
        .filter(|(_, record)| record["kind"] == "budget-blocked")
        .map(|(index, _)| index)
        .collect();
    assert_eq!(blocked.len(), 1, "one budget-blocked record: {blocked:?}");
    let off = records
        .iter()
        .find(|record| record["kind"] == "commander-off" && record["data"]["reason"] == OFF_BUDGET)
        .unwrap_or_else(|| panic!("the commander off for the budget: {}", landed.report_text));
    assert_eq!(at_block, 3, "the stub had heard three at the block");
    assert_eq!(
        total, at_block,
        "no request reached the stub after the block"
    );

    // The request ids: three admitted, each model record under its own.
    let ids: Vec<u64> = records
        .iter()
        .filter(|record| record["kind"] == "model-request")
        .map(|record| record["data"]["id"].as_u64().expect("a request id"))
        .collect();
    assert_eq!(ids.len(), 3, "three requests admitted: {ids:?}");
    for record in records
        .iter()
        .filter(|record| MODEL_RECORDS.contains(&record["kind"].as_str().unwrap_or("")))
    {
        let id = record["request_id"].as_u64();
        assert!(
            id.is_some_and(|id| ids.contains(&id)),
            "a {} record under {}, not one of {ids:?}",
            record["kind"],
            record["request_id"]
        );
    }
    // …and each request's own records under it, and no other's: the two
    // answered calls each its input, request, reply, usage and receipt; the
    // held third its input, request and cancel, and no reply. (Call 3 is asked
    // only after call 2's answer, so a reply filed under the wrong request
    // shows here as a count off by one.)
    for (n, id) in ids.iter().enumerate() {
        let answered = usize::from(n < 2);
        let expected = [1, 1, answered, answered, answered, 1 - answered];
        let counts: Vec<usize> = MODEL_RECORDS
            .iter()
            .map(|kind| {
                records
                    .iter()
                    .filter(|record| record["kind"] == *kind && record["request_id"] == *id)
                    .count()
            })
            .collect();
        assert_eq!(
            counts,
            expected,
            "request {} (id {id}): {MODEL_RECORDS:?} filed under it",
            n + 1
        );
    }
    for record in records
        .iter()
        .filter(|record| record["kind"] == "model-request")
    {
        assert_eq!(
            record["request_id"], record["data"]["id"],
            "a model-request under its own id"
        );
    }

    // The cancel: the third request, by the abort or the stale bound.
    let third = ids[2];
    let asked_at = records
        .iter()
        .position(|record| record["kind"] == "model-request" && record["data"]["id"] == third)
        .expect("the third model-request");
    let cancelled_at = records
        .iter()
        .position(|record| {
            record["kind"] == "model-cancelled"
                && record["data"]["request_id"] == third
                && record["request_id"] == third
        })
        .unwrap_or_else(|| panic!("the third request cancelled: {}", landed.report_text));
    assert!(cancelled_at > asked_at);
    let by = if records[asked_at..cancelled_at]
        .iter()
        .any(|record| record["kind"] == "commander-stale" && record["data"]["abandoned"] == true)
    {
        "the lane's stale bound"
    } else {
        "the abort"
    };
    let named = matches_named(records);
    let block_match = off["data"]["match"].as_u64();
    let block_in = match named.iter().position(|id| Some(*id) == block_match) {
        Some(0) => "match 1",
        Some(1) => "match 2",
        _ => "neither match",
    };

    // The ledger: every request the stub heard, plus at most one cancelled
    // before it reached it; at least one cancelled; no goal decision.
    let filed = &landed.report["costs"]["session"]["by_prompt_type"]["arena_commander"];
    let requests = filed["requests"].as_u64().expect("requests");
    let cancelled = filed["cancelled"].as_u64().unwrap_or(0);
    let heard_n = u64::try_from(total).expect("a count");
    assert!(
        requests >= heard_n && requests <= heard_n + 1,
        "{requests} filed against {heard_n} heard: {filed}"
    );
    assert!(cancelled >= 1, "the third call filed cancelled: {filed}");
    assert!(
        landed.report["costs"]["session"]["by_prompt_type"]["goal_decision"].is_null(),
        "no goal turn was asked"
    );
    assert_eq!(
        landed.report["state"]["model_calls"], 0,
        "no commander call counted as a goal call: {}",
        landed.report_text
    );

    // The pilot flew on without the commander: a `FIRE` in match 2 after the
    // block.
    assert!(
        blocked_then_fired(records).is_some_and(|(at, _)| at == blocked[0]),
        "a FIRE in match 2 after the block"
    );

    // Two matches, both called off.
    let matches = landed.matches();
    assert_eq!(matches.len(), 2, "{}", landed.report_text);
    for flown in matches {
        assert_eq!(flown["result"], "abort", "{}", landed.report_text);
    }
    // The budget before the ask: the matches' commander calls are the three
    // requests admitted, so the ask the budget refused was never counted as a
    // call (a runner that took the ask before the budget would count four).
    let calls: u64 = matches
        .iter()
        .map(|flown| flown["commander_calls"].as_u64().expect("commander_calls"))
        .sum();
    assert_eq!(
        calls,
        u64::try_from(ids.len()).expect("a count"),
        "the matches' commander calls are the admitted requests: {}",
        landed.report_text
    );
    assert_eq!(landed.report["success"], false);
    assert_eq!(landed.code, Some(2), "stderr:\n{}", landed.stderr);
    println!(
        "COMMANDER THROUGH THE RUNNER: the stub heard {total}; the third call was cancelled by \
         {by}; the budget blocked in {block_in}; {requests} filed ({cancelled} cancelled); \
         wall {:.1} s",
        wall.as_secs_f64()
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

/// **A refused pose answers at once** (cut AK.3): `arena_rig::try_pose` ends
/// its wait on the verb's own refusal sentence rather than sitting out the
/// whole `STEP_TIMEOUT`, so a posed lane that re-poses before every match
/// loses a moment, not twenty seconds and its match's opening, to a refusal.
/// A cell off the window, a heading out of range and (cut AM.3) a callsign
/// the centre has no slot for each come back as `Err` with the verb's
/// sentence well inside a quarter of the step timeout (the old wait, which
/// knew only `opens at cell`, took all of it); a good pose still comes back
/// `Ok`.
///
/// **And a closed session ends every wait at once** (cut AM.3), on the same
/// rig: God types `QUIT`, the server's own disconnect; a pose typed straight
/// behind it, whose wait meets the close, is an `Err` saying the session is
/// closed (inside 10 s); once the channel has closed (read for 10 s at most),
/// `try_pose` is an `Err` saying the session is closed, and `try_await_line` over a flight whose bot never ends is
/// `Err(Ended::Closed)`, each inside 2 s and neither a panic.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_refused_pose_answers_at_once() {
    let _one_world = ARENA.lock().await;
    tokio::time::timeout(Duration::from_secs(90), refuse_poses())
        .await
        .expect("the poses are answered inside 90 s");
}

async fn refuse_poses() {
    let rig = arena_rig::boot(arena_rig::arena_config(), &[("pilot", "Pilot")]).await;
    let mut god =
        arena_rig::Client::login(&rig, arena_rig::GOD_ACCOUNT, arena_rig::GOD_PASSWORD, "God")
            .await;
    for (spec, sentence) in [
        ("RA=5000 5000 90", "is off the arena window."),
        ("RA=100 97 400", "Heading is 0 to 359."),
        ("ZZ=100 97 90", "There is no slot ZZ on this centre."),
    ] {
        let asked = Instant::now();
        let answer = arena_rig::try_pose(&mut god, spec).await;
        let took = asked.elapsed();
        let text = answer.expect_err(spec);
        assert!(text.contains(sentence), "{spec}: {text}");
        assert!(
            took < arena_rig::STEP_TIMEOUT / 4,
            "{spec} was refused after {took:?}, not at once: {text}"
        );
    }
    let text = arena_rig::try_pose(&mut god, "RA=100 97 90")
        .await
        .expect("a pose on the road is taken");
    assert!(text.contains("opens at cell"), "{text}");

    // The closing tail: God quits, and every wait on the closed session ends
    // at once.
    god.quit().await;
    // A pose typed straight behind `QUIT`, before the close was read, goes
    // out (or is refused by the dying channel) and its wait meets the close:
    // that too is the closed session's `Err`, never an empty or stray answer.
    let asked = Instant::now();
    let text = arena_rig::try_pose(&mut god, "RA=100 97 90")
        .await
        .expect_err("a pose behind QUIT is refused");
    let took = asked.elapsed();
    assert!(text.contains("the session is closed"), "{text}");
    assert!(took < Duration::from_secs(10), "refused after {took:?}");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !god.closed && Instant::now() < deadline {
        god.pump(deadline.saturating_duration_since(Instant::now()))
            .await;
    }
    assert!(
        god.closed,
        "the session closed after QUIT:\n{}",
        god.drain()
    );
    let asked = Instant::now();
    let text = arena_rig::try_pose(&mut god, "RA=100 97 90")
        .await
        .expect_err("a pose on a closed session is refused");
    let took = asked.elapsed();
    assert!(text.contains("the session is closed"), "{text}");
    assert!(took < Duration::from_secs(2), "refused after {took:?}");
    // A flight whose bot never ends: only the closed watcher can end the
    // wait (the paths are never read).
    let mut flight = arena_rig::Flight {
        task: tokio::spawn(std::future::pending()),
        transcript: rig.dir.join("never.jsonl"),
        report: rig.dir.join("never.json"),
    };
    // From the end of what was read, so no earlier text can hold the needle.
    let from = god.seen.len();
    let asked = Instant::now();
    let ended = arena_rig::try_await_line(
        &mut god,
        from,
        "never",
        &mut flight,
        Duration::from_secs(60),
    )
    .await;
    let took = asked.elapsed();
    assert_eq!(ended, Err(arena_rig::Ended::Closed));
    assert!(took < Duration::from_secs(2), "ended after {took:?}");
    assert!(!flight.task.is_finished(), "the bot's task was never read");
    flight.task.abort();
    god.close().await;
    drop(rig);
}

// ---------------------------------------------------------------------------
// Cut AM.2: the prompt while it flies
// ---------------------------------------------------------------------------

/// Every whole record of a transcript still being written (a torn last line
/// is left until it is whole).
fn records_now(path: &std::path::Path) -> Vec<Value> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.split_inclusive('\n')
        .filter(|line| line.ends_with('\n'))
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// Where the first record of `kind` whose `data` is `data` sits, from `from`.
fn position_of(records: &[Value], from: usize, kind: &str, data: &str) -> Option<usize> {
    records
        .iter()
        .enumerate()
        .skip(from)
        .find(|(_, record)| record["kind"] == kind && record["data"] == data)
        .map(|(index, _)| index)
}

/// Write one control line to the bot's prompt.
async fn type_at_prompt(stdin: &mut tokio::process::ChildStdin, line: &str) {
    use tokio::io::AsyncWriteExt;
    stdin
        .write_all(format!("{line}\n").as_bytes())
        .await
        .expect("the control line is written");
    stdin.flush().await.expect("the control line is flushed");
}

/// **The prompt pauses and resumes a flight** (cut AM.2; wall clock,
/// generously bounded at 120 s). The bot flies alone against one machine
/// posed 500 m off (as [`fly_alone`]) at its prompt, not headless, with
/// `[agent] settle_ms = 15000` and `observation_ms = 20000`: a command's
/// output is finished only once quiet for 15 s or at 20 s, and the lane's own
/// polls never let it go quiet — so before the cut the pilot typed nothing for
/// up to 20 s after `/resume`. After its first `FIRE` (and 21 s after the
/// launch, when the login's own output is finished) the person types
/// `/send STATUS` (again each second while the bot is still collecting
/// output), then `/resume` once the `STATUS` has gone. Nothing of the lane's
/// is typed between the two; the pause and the resume are said, in order; the
/// lane types again within 5 s of `/resume`; and `STATUS`'s own
/// `command-result` comes after that first line — the pending command was
/// still open while the lane typed. God then calls the match off, and the
/// report holds the one match as `abort`. No model is configured.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_repl_pauses_and_resumes_a_flight() {
    let _one_world = ARENA.lock().await;
    tokio::time::timeout(Duration::from_secs(120), pause_and_resume())
        .await
        .expect("the paused flight finishes inside 120 s");
}

async fn pause_and_resume() {
    let mut cfg = arena_rig::arena_config();
    cfg.match_.startup_ms = 2_000;
    cfg.match_.board_timeout_ms = 60_000;
    cfg.match_.max_duration_ms = 100_000;
    let rig = arena_rig::boot(cfg, &[("pilot", "Pilot")]).await;
    let mut god =
        arena_rig::Client::login(&rig, arena_rig::GOD_ACCOUNT, arena_rig::GOD_PASSWORD, "God")
            .await;
    arena_rig::pose(&mut god, "RA=100 97 90").await;
    arena_rig::pose(&mut god, "BA=150 97 270").await;
    arena_rig::walk_to_lobby(&mut god).await;

    let config = arena_rig::bot_config_agent(&rig, "pilot+Pilot", 150, 15_000, 20_000, "");
    let from = god.seen.len();
    let launched = Instant::now();
    let (mut flight, mut stdin) =
        arena_rig::launch_interactive(&rig, &config, "repl", &["--arena", "red"]);
    let hunting = arena_rig::await_line(
        &mut god,
        from,
        "Good hunting",
        &mut flight,
        Duration::from_secs(60),
    )
    .await;
    assert!(
        arena_rig::await_fire(&mut god, &mut flight, Duration::from_secs(40)).await,
        "the pilot fired"
    );
    while launched.elapsed() < Duration::from_secs(21) {
        god.idle(Duration::from_millis(250)).await;
    }

    // `/send STATUS`, again each second while the bot is still collecting
    // output, until its `send` event is in the transcript.
    let mut sent = false;
    for _ in 0..30 {
        let errors = records_now(&flight.transcript)
            .iter()
            .filter(|record| record["kind"] == "control-error")
            .count();
        type_at_prompt(&mut stdin, "/send STATUS").await;
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut refused = false;
        while Instant::now() < deadline {
            let records = records_now(&flight.transcript);
            if position_of(&records, 0, "send", "STATUS").is_some() {
                sent = true;
                break;
            }
            if records
                .iter()
                .filter(|record| record["kind"] == "control-error")
                .count()
                > errors
            {
                refused = true;
                break;
            }
            god.idle(Duration::from_millis(50)).await;
        }
        if sent {
            break;
        }
        assert!(refused, "`/send STATUS` was neither sent nor refused");
        god.idle(Duration::from_secs(1)).await;
    }
    assert!(sent, "`/send STATUS` never went");
    type_at_prompt(&mut stdin, "/resume").await;

    // Ten of the lane's lines after the `/resume` record, or 30 s.
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let records = records_now(&flight.transcript);
        let after = position_of(&records, 0, "control", "/resume").map_or(0, |at| {
            records[at..]
                .iter()
                .filter(|record| record["kind"] == "arena-send")
                .count()
        });
        if after >= 10 || Instant::now() >= deadline {
            break;
        }
        assert!(!flight.task.is_finished(), "the bot ended mid-flight");
        god.idle(Duration::from_millis(50)).await;
    }
    arena_rig::still_running(&god, hunting);
    god.say("@arena/abort", "is stopping").await;

    // The abort read, and `STATUS`'s own result written (at its 20 s
    // deadline at the latest), before `/quit`.
    let deadline = Instant::now() + Duration::from_secs(40);
    loop {
        let records = records_now(&flight.transcript);
        let aborted = records.iter().any(|record| {
            record["kind"] == "arena"
                && record["data"]
                    .as_str()
                    .is_some_and(|text| text.contains(" is over (abort)"))
        });
        let finished = records.iter().any(|record| {
            record["kind"] == "command-result" && record["data"]["command"] == "STATUS"
        });
        if (aborted && finished) || Instant::now() >= deadline {
            break;
        }
        god.idle(Duration::from_millis(100)).await;
    }
    type_at_prompt(&mut stdin, "/quit").await;
    let landed = arena_rig::land(flight, Duration::from_secs(30)).await;
    drop(stdin);

    let records = &landed.records;
    let send_at = position_of(records, 0, "control", "/send STATUS").expect("the /send record");
    let resume_at =
        position_of(records, send_at, "control", "/resume").expect("the /resume record");
    assert!(
        !records[send_at..resume_at]
            .iter()
            .any(|record| record["kind"] == "arena-send"),
        "the lane typed while paused"
    );
    let said = landed.events("arena");
    let paused = said
        .iter()
        .position(|text| text == "Paused: typing nothing until /resume; still reading.")
        .unwrap_or_else(|| panic!("the pause is said: {said:?}"));
    let resumed = said
        .iter()
        .position(|text| {
            text.starts_with("Resumed after ")
                && text.ends_with(" s; every wait starts again from now.")
        })
        .unwrap_or_else(|| panic!("the resume is said: {said:?}"));
    assert!(paused < resumed, "{said:?}");
    let first = records
        .iter()
        .enumerate()
        .skip(resume_at)
        .find(|(_, record)| record["kind"] == "arena-send")
        .map(|(index, _)| index)
        .expect("the lane typed after /resume");
    let resume_ms = records[resume_at]["elapsed_ms"].as_u64().expect("elapsed");
    let first_ms = records[first]["elapsed_ms"].as_u64().expect("elapsed");
    let result = records
        .iter()
        .position(|record| {
            record["kind"] == "command-result" && record["data"]["command"] == "STATUS"
        })
        .expect("STATUS's command-result");
    let result_ms = records[result]["elapsed_ms"].as_u64().expect("elapsed");
    let send_ms = records[send_at]["elapsed_ms"].as_u64().expect("elapsed");
    println!(
        "REPL: /send STATUS at {send_ms} ms, /resume at {resume_ms} ms, first arena-send {} ms \
         after it ({}), STATUS's command-result at {result_ms} ms ({} ms after the /resume)",
        first_ms - resume_ms,
        records[first]["data"]["line"],
        result_ms.saturating_sub(resume_ms)
    );
    assert!(
        first_ms <= resume_ms + 5_000,
        "the first line {} ms after /resume",
        first_ms - resume_ms
    );
    assert!(
        result > first,
        "STATUS's command-result before the lane typed again: the pending command held the pilot"
    );
    let matches = landed.matches();
    assert_eq!(matches.len(), 1, "{}", landed.report_text);
    assert_eq!(matches[0]["result"], "abort", "{}", landed.report_text);
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

// ---------------------------------------------------------------------------
// Cut AN.2: the opening from the shipped muster
// ---------------------------------------------------------------------------

/// **The opening reads the ground at the muster** (cut AN.2; wall clock,
/// bounded at 180 s). The rig with the AI fill and **no pose**: the shipped
/// muster, red's slot 1 on the east-west road. The bot flies `--arena red
/// --matches 1` with the default `[arena]` (the opening waits, the
/// re-approach is on) on a 180 s cap. Once it has said `Opening: holding near
/// our entry` and written the next `arena-picture`, God calls the match off.
/// It typed `VIEW BRIEF` exactly twice (cut AO.1: red's first pod musters
/// about 65 m off the west edge, so the pilot first moves in for room behind,
/// says so, types `SPEED 0` and reads the brief again); it said it was
/// stepping off the road to the north or the south after the move in and
/// before it held; and that picture's own position lies at least 20 m north
/// or south of where the step began (the last picture written after the move
/// in and before the step was said: the step is chosen from the second
/// brief's position, not the start the pilot kept), the step being done
/// once 25 m of it is behind it, and at least
/// `OPENING_ROOM_M` less 40 m east of the west edge (red's room behind is its
/// own x). This is the format contract of the brief's bearing lines
/// against the game's own `brief_lines`: the step is chosen from them alone.
/// The report holds the one match as `abort`. No model is configured.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_opening_reads_the_ground_at_the_muster() {
    let _one_world = ARENA.lock().await;
    tokio::time::timeout(Duration::from_secs(180), open_at_the_muster())
        .await
        .expect("the opening finishes inside 180 s");
}

async fn open_at_the_muster() {
    let mut cfg = arena_rig::arena_config();
    cfg.match_.startup_ms = 2_000;
    cfg.match_.board_timeout_ms = 60_000;
    cfg.match_.max_duration_ms = 180_000;
    let rig = arena_rig::boot(cfg, &[("pilot", "Pilot")]).await;
    let mut god =
        arena_rig::Client::login(&rig, arena_rig::GOD_ACCOUNT, arena_rig::GOD_PASSWORD, "God")
            .await;
    arena_rig::walk_to_lobby(&mut god).await;
    let config = arena_rig::bot_config(&rig, "pilot+Pilot", "");
    let started = Instant::now();
    let flight = arena_rig::launch(
        &rig,
        &config,
        "opening",
        &["--arena", "red", "--matches", "1"],
    );

    // The hold said, then the next picture record (one every 5 s).
    let held = |record: &Value| {
        record["kind"] == "arena"
            && record["data"]
                .as_str()
                .is_some_and(|text| text.starts_with("Opening: holding near our entry"))
    };
    let deadline = Instant::now() + Duration::from_secs(140);
    let picture = loop {
        let records = records_now(&flight.transcript);
        if let Some(at) = records.iter().position(held)
            && let Some(picture) = records
                .iter()
                .skip(at + 1)
                .find(|record| record["kind"] == "arena-picture")
        {
            break picture.clone();
        }
        assert!(
            Instant::now() < deadline,
            "no hold and picture after it: {:?}",
            records
                .iter()
                .filter(|record| record["kind"] == "arena")
                .map(|record| record["data"].clone())
                .collect::<Vec<_>>()
        );
        assert!(!flight.task.is_finished(), "the bot ended before its hold");
        god.idle(Duration::from_millis(200)).await;
    };
    let held_after = started.elapsed();
    god.say("SIM ABORT", "aborts match").await;
    let landed = arena_rig::land(flight, Duration::from_secs(60)).await;

    let sends = landed.sends();
    for (line, _, _) in &sends {
        assert!(allowed(line), "`{line}` is not the pilot's to type");
        assert!(!forbidden(line), "`{line}` is forbidden");
    }
    let briefs: Vec<&(String, String, u64)> = sends
        .iter()
        .filter(|(line, _, _)| line == "VIEW BRIEF")
        .collect();
    // Cut AO.1: one at the muster and one after the move in for room.
    assert_eq!(briefs.len(), 2, "two VIEW BRIEFs: {sends:?}");
    assert!(briefs.iter().all(|brief| brief.1 == "poll"), "{briefs:?}");
    // The move in said, then a `SPEED 0` typed, then the second brief, in
    // the transcript's own order.
    let records = &landed.records;
    let moving_in = records
        .iter()
        .position(|record| {
            record["kind"] == "arena"
                && record["data"]
                    .as_str()
                    .is_some_and(|text| text.starts_with("Opening: moving in "))
        })
        .unwrap_or_else(|| panic!("the move in: {:?}", landed.events("arena")));
    let second_brief = records
        .iter()
        .enumerate()
        .filter(|(_, record)| {
            record["kind"] == "arena-send" && record["data"]["line"] == "VIEW BRIEF"
        })
        .nth(1)
        .map(|(at, _)| at)
        .expect("the second brief");
    assert!(
        records
            .get(moving_in..second_brief)
            .is_some_and(|between| between.iter().any(|record| {
                record["kind"] == "arena-send" && record["data"]["line"] == "SPEED 0"
            })),
        "a stop between the move in and the second brief: {sends:?}"
    );
    // Where the step began: the last picture written after the move in and
    // before the step was said. The tank is stopped there for the second
    // brief (or still slowing along the east-west road), so its y is the
    // step's own origin, the leg's drift off the road included.
    let stepping = records
        .iter()
        .position(|record| {
            record["kind"] == "arena"
                && record["data"].as_str().is_some_and(|text| {
                    text.starts_with("Opening: stepping off the road to the north (")
                        || text.starts_with("Opening: stepping off the road to the south (")
                })
        })
        .unwrap_or_else(|| panic!("the step: {:?}", landed.events("arena")));
    let step_from = records
        .get(moving_in..stepping)
        .and_then(|between| {
            between
                .iter()
                .rev()
                .find(|record| record["kind"] == "arena-picture")
        })
        .and_then(|record| record["data"]["own"]["pos"].as_array())
        .and_then(|pos| Some((pos.first()?.as_f64()?, pos.get(1)?.as_f64()?)))
        .unwrap_or_else(|| panic!("a picture between the move in and the step: {sends:?}"));
    let said = landed.events("arena");
    let step = said
        .iter()
        .position(|text| {
            text.starts_with("Opening: stepping off the road to the north (")
                || text.starts_with("Opening: stepping off the road to the south (")
        })
        .unwrap_or_else(|| panic!("a step north or south: {said:?}"));
    let hold = said
        .iter()
        .position(|text| text.starts_with("Opening: holding near our entry"))
        .unwrap_or_else(|| panic!("the hold: {said:?}"));
    assert!(step < hold, "{said:?}");
    let moved = said
        .iter()
        .position(|text| text.starts_with("Opening: moving in "))
        .unwrap_or_else(|| panic!("the move in: {said:?}"));
    assert!(moved < step, "{said:?}");
    let pilot = &picture["data"]["pilot"];
    let start = pilot["start_m"]
        .as_array()
        .unwrap_or_else(|| panic!("the start kept: {pilot}"));
    let own = picture["data"]["own"]["pos"]
        .as_array()
        .unwrap_or_else(|| panic!("the own position: {picture}"));
    let (start_x, start_y) = (start[0].as_f64().expect("x"), start[1].as_f64().expect("y"));
    let (own_x, own_y) = (own[0].as_f64().expect("x"), own[1].as_f64().expect("y"));
    println!(
        "OPENING: {} then {} then {}; start ({start_x}, {start_y}), step from ({:.0}, \
         {:.0}), held at ({own_x}, {own_y}); the hold read {:.1} s after the launch",
        said[moved],
        said[step],
        said[hold],
        step_from.0,
        step_from.1,
        held_after.as_secs_f64()
    );
    // The step is measured from its own origin (cut AO.1: after the move in,
    // not the muster the pilot kept).
    assert!(
        (own_y - step_from.1).abs() >= 20.0,
        "stepped off the road: start ({start_x}, {start_y}), step from {step_from:?}, now \
         ({own_x}, {own_y})"
    );
    // Cut AO.1: red's room behind is its own x, at least the room the pilot
    // wants less the arrival and the brief's rounding.
    assert!(
        own_x >= mush_bot::arena::pilot::OPENING_ROOM_M - 40.0,
        "held with room behind: start ({start_x}, {start_y}), now ({own_x}, {own_y})"
    );
    assert_eq!(pilot["opening"], "hold", "{pilot}");
    let matches = landed.matches();
    assert_eq!(matches.len(), 1, "{}", landed.report_text);
    assert_eq!(matches[0]["result"], "abort", "{}", landed.report_text);
    god.close().await;
    drop(rig);
}

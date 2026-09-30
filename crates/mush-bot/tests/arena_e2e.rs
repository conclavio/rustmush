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
//! Every arena test in this file holds one file-wide async mutex, so two arena
//! worlds never run at once in this binary.
use std::{sync::LazyLock, time::Duration};

use mush_bot::arena::{Line, classify};
use serde_json::Value;

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

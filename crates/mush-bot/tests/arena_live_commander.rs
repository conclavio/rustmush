//! **The live commander lane** (cut AK.3, 16c Q-AK4): the win-rate lane's
//! rig, poses and 420 s cap (the rig's `fly_lane`, which every lane calls),
//! five matches, `--arena red --matches 5`, with `bots/arena/commander.toml`'s
//! own commander and its **real model**.
//!
//! **From the muster** (cut AN.1): with `MUSH_BOT_LIVE_FROM_MUSTER=1` as well,
//! it flies the muster lane's match instead — no pose, the shipped muster, a
//! 600 s cap — on a 90-minute deadline; unset or `0` flies posed as before,
//! and anything else panics before the launch ([`from_muster`]). **The cost**:
//! the commander asks every `commander_ms` whatever the picture holds, and the
//! live lane read posed about one call each two seconds (31 to 97 calls in 60
//! to 196 s matches). From the muster a match of up to 600 s, much of it with
//! nothing in sight, costs up to about 300 calls, under the 400 a match that
//! `commander_max_calls` allows, and five matches up to about 1 500. It prints
//! first `FLOWN FROM: the muster (600 s cap)` or `FLOWN FROM: posed 500 m
//! apart (420 s cap)`.
//!
//! **It spends real money, so nothing runs it but the lead, by hand.** It is
//! `#[ignore]`, and even under `--ignored` it only prints
//! `skipped: set MUSH_BOT_LIVE_COMMANDER=1 and export OPENROUTER_API_KEY to fly
//! five matches with bots/arena/commander.toml's model` and passes unless
//! `MUSH_BOT_LIVE_COMMANDER` is `1`. Opted in, it **fails** rather than skips
//! when `OPENROUTER_API_KEY` is missing or empty in the test's own process
//! environment (`cargo test` does not read `.env`), so the lead's one paid run
//! can never be a silent green. The key goes to the bot child only, through
//! the rig's `launch_env`; it is never printed, logged or written to a file.
//! No CI job runs it and CI never sets either variable: `.github/workflows/ci.yml`
//! names it in a comment in the `bot-win-rate` job, which is what the gated-test
//! inventory asks of every ignored test. The lead runs
//!
//! ```text
//! export OPENROUTER_API_KEY=…
//! MUSH_BOT_LIVE_COMMANDER=1 cargo test -p mush-bot --test arena_live_commander -- --ignored --nocapture
//! ```
//!
//! and, to fly it from the muster,
//!
//! ```text
//! MUSH_BOT_LIVE_COMMANDER=1 MUSH_BOT_LIVE_FROM_MUSTER=1 cargo test -p mush-bot --test arena_live_commander -- --ignored --nocapture
//! ```
//!
//! After `FLOWN FROM`, it prints `LIVE COMMANDER: {w} of 5 for red against one
//! machine at 1 v 1 (…)`, one line a match with the commander's tally, the
//! win-rate lane's reading lines (cut AL.3: the rig's `Reading::lines`,
//! `ENGAGEMENT` with the stances flown, `FIRE GAP`, `SHOT GAP`, `STILL
//! CYCLING`, `HITS TAKEN BY FACET`, and cut AN.1's `SEEN FIRST`, `OPENING` and
//! `FRONT SHARE`), and one `COST:` line from the report's cost ledger
//! ([`live_lines`]), and asserts only that five matches finished. `MUSH_BOT_WIN_RATE_KEEP` keeps `live-commander.jsonl` and
//! `live-commander.json` as it keeps the win-rate lane's.
//!
//! **Its setup is proved offline**, with no key and no model:
//! [`the_live_lane_config_loads_and_resolves`] loads the config the lane
//! writes ([`live_commander_extra`]) through the bot's own `Config::load`,
//! resolves its commander from the table, formats a hand-written report
//! through [`live_lines`], and reads `MUSH_BOT_LIVE_FROM_MUSTER`'s three
//! answers through [`from_muster`].
use std::time::Duration;

use mush_bot::{
    arena::commander::{self, COMMANDER_TIMEOUT_MAX_S},
    config::Config,
};
use serde_json::{Value, json};

#[path = "support/arena.rs"]
mod arena_rig;

/// The matches the live lane flies.
const LIVE_MATCHES: usize = 5;
/// The whole flight's deadline, from the launch: five matches of 420 s with
/// boarding and startup are about 38 minutes.
const DEADLINE: Duration = Duration::from_secs(60 * 60);
/// The deadline from the muster (cut AN.1): five matches of up to 600 s with
/// boarding and startup are about 55 minutes.
const MUSTER_DEADLINE: Duration = Duration::from_secs(90 * 60);
/// The variable that opts the lead in.
const OPT_IN: &str = "MUSH_BOT_LIVE_COMMANDER";
/// The variable that flies the lane from the muster (cut AN.1).
const FROM_MUSTER: &str = "MUSH_BOT_LIVE_FROM_MUSTER";

/// **Whether the live lane flies from the muster** (cut AN.1), from
/// [`FROM_MUSTER`]'s value: unset or `0` → posed, `1` → the muster, and
/// anything else a panic, before anything is launched.
fn from_muster(value: Option<&str>) -> bool {
    match value {
        None | Some("0") => false,
        Some("1") => true,
        Some(other) => panic!("MUSH_BOT_LIVE_FROM_MUSTER is 0 or 1, not {other}"),
    }
}
/// The commander's key, as `bots/arena/commander.toml` names it.
const KEY_ENV: &str = "OPENROUTER_API_KEY";

/// `bots/arena/commander.toml`, the config the live lane's commander is read
/// from.
const COMMANDER_TOML: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../bots/arena/commander.toml"
);

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "spends real model money: the lead runs it by hand with MUSH_BOT_LIVE_COMMANDER=1 and OPENROUTER_API_KEY exported (16c Q-AK4); no CI job runs it"]
async fn arena_live_commander_five_matches() {
    if std::env::var("MUSH_BOT_LIVE_COMMANDER").as_deref() != Ok("1") {
        println!(
            "skipped: set {OPT_IN}=1 and export {KEY_ENV} to fly five matches with bots/arena/commander.toml's model"
        );
        return;
    }
    let key = std::env::var("OPENROUTER_API_KEY").unwrap_or_default();
    let key = key.trim();
    if key.is_empty() {
        panic!(
            "{OPT_IN}=1 but {KEY_ENV} is not in this process's environment (cargo test does not read .env): export it and run again"
        );
    }

    // Read after the opt-in, so the skip path is unchanged (cut AN.1).
    let value = std::env::var_os(FROM_MUSTER).map(|v| v.to_string_lossy().into_owned());
    let muster = from_muster(value.as_deref());

    let extra = live_commander_extra();
    let env = [(KEY_ENV, key)];
    let plan = if muster {
        arena_rig::LanePlan {
            name: "live-commander",
            matches: LIVE_MATCHES,
            deadline: MUSTER_DEADLINE,
            extra: &extra,
            env: &env,
            poses: &[],
            max_duration_ms: arena_rig::MUSTER_DURATION_MS,
            max_seconds: arena_rig::MUSTER_MAX_SECONDS,
        }
    } else {
        arena_rig::LanePlan {
            name: "live-commander",
            matches: LIVE_MATCHES,
            deadline: DEADLINE,
            extra: &extra,
            env: &env,
            poses: &arena_rig::POSES,
            max_duration_ms: arena_rig::MAX_DURATION_MS,
            max_seconds: arena_rig::MAX_SECONDS,
        }
    };
    // First, before anything the flight prints (cut AN.1).
    println!(
        "{}",
        if muster {
            "FLOWN FROM: the muster (600 s cap)"
        } else {
            "FLOWN FROM: posed 500 m apart (420 s cap)"
        }
    );
    let posed = arena_rig::fly_lane(&plan).await;

    // The lead's reading (cut AL.3), after the match lines and before the cost.
    let reading = arena_rig::reading(
        &posed.partial.transcript_text,
        posed.partial.report.as_ref(),
    )
    .lines(&live_engagement());
    match &posed.partial.report {
        Some(report) => {
            let unposed = posed.unposed();
            let lines = live_lines(report);
            // The first line is the results, the last the cost; between them
            // one line a match, in the report's order.
            for (index, line) in lines.iter().enumerate() {
                if index + 1 == lines.len() {
                    for read in &reading {
                        println!("{read}");
                    }
                }
                let flagged = index
                    .checked_sub(1)
                    .and_then(|at| posed.flown().get(at))
                    .and_then(|flown| flown["match"].as_u64())
                    .is_some_and(|id| unposed.contains(&id))
                    && line.starts_with("match ");
                println!("{line}{}", if flagged { " (unposed)" } else { "" });
            }
        }
        None => {
            println!("{}", posed.outcome().line("LIVE COMMANDER", LIVE_MATCHES));
            for read in &reading {
                println!("{read}");
            }
            println!("{}", arena_rig::NO_REPORT);
        }
    }
    if let Some(line) = posed.unposed_line() {
        println!("{line}");
    }
    if let Some(line) = posed.early_line() {
        println!("{line}");
    }
    assert_eq!(
        posed.flown().len(),
        LIVE_MATCHES,
        "five matches finished: {}\nstderr:\n{}",
        posed
            .partial
            .report_text
            .as_deref()
            .unwrap_or("there was no report"),
        posed.partial.stderr
    );
}

/// **The live lane's extra config tables**: `bots/arena/commander.toml`'s
/// whole `[arena]` table (`commander = true`, its cadence and call cap, and
/// `[arena.commander_model]` with its `reasoning` and `provider`), with
/// `lead_start_grace_ms = 5000` as the posed lanes fly, serialised as **one
/// value** `{arena = …}`. Serialised so, `reasoning` and `provider` come out as
/// `[arena.commander_model.reasoning]` and `[arena.commander_model.provider]`;
/// the inner table serialised alone under an `[arena]` header would make them
/// top-level tables, which `Config` refuses (`deny_unknown_fields`).
fn live_commander_extra() -> String {
    let text = std::fs::read_to_string(COMMANDER_TOML).expect("bots/arena/commander.toml");
    let file: toml::Table = toml::from_str(&text).expect("commander.toml is TOML");
    let mut arena = file
        .get("arena")
        .and_then(toml::Value::as_table)
        .cloned()
        .expect("commander.toml's [arena] table");
    arena.insert("lead_start_grace_ms".into(), toml::Value::Integer(5_000));
    let mut outer = toml::Table::new();
    outer.insert("arena".into(), toml::Value::Table(arena));
    format!(
        "\n{}",
        toml::to_string(&outer).expect("the [arena] table serialises")
    )
}

/// **The engagement the live lane flies** (cut AL.3): `bots/arena/commander.toml`'s
/// `[arena] engagement`, `hold` (the default) when it names none.
fn live_engagement() -> String {
    let text = std::fs::read_to_string(COMMANDER_TOML).expect("bots/arena/commander.toml");
    let file: toml::Table = toml::from_str(&text).expect("commander.toml is TOML");
    file.get("arena")
        .and_then(|arena| arena.get("engagement"))
        .and_then(toml::Value::as_str)
        .unwrap_or("hold")
        .to_owned()
}

/// A count from the report, 0 when absent.
fn count(value: &Value) -> u64 {
    value.as_u64().unwrap_or(0)
}

/// **What the live lane prints from the report**: the results line (AJ's
/// `WIN RATE` format under `LIVE COMMANDER`), one line a match with the
/// commander's tally, and the cost of every `arena_commander` call in the
/// session (`costs.session.by_prompt_type.arena_commander`; the US dollars
/// are the provider's own `provider_cost_by_currency.USD`, `unknown` when
/// calls were made and none was priced).
fn live_lines(report: &Value) -> Vec<String> {
    let mut lines =
        vec![arena_rig::Outcome::from_report(report).line("LIVE COMMANDER", LIVE_MATCHES)];
    let matches = report["arena"]["matches"]
        .as_array()
        .map_or(&[][..], Vec::as_slice);
    for flown in matches {
        lines.push(format!(
            "match {}: {}; {} shots, {} hits, took {}; {} s; commander {} calls, {} applied, {} late, {} overruled, {} invalid, {} errors, off: {}",
            count(&flown["match"]),
            flown["result"].as_str().unwrap_or("?"),
            count(&flown["shots"]),
            count(&flown["hits_scored"]),
            count(&flown["hits_taken"]),
            count(&flown["duration_ms"]) / 1_000,
            count(&flown["commander_calls"]),
            count(&flown["commander_applied"]),
            count(&flown["commander_stale"]),
            count(&flown["commander_overruled"]),
            count(&flown["commander_invalid"]),
            count(&flown["commander_errors"]),
            flown["commander_off"].as_str().unwrap_or("none"),
        ));
    }
    let filed = &report["costs"]["session"]["by_prompt_type"]["arena_commander"];
    let requests = count(&filed["requests"]);
    let usd = match &filed["provider_cost_by_currency"]["USD"] {
        Value::String(amount) => amount.clone(),
        Value::Number(amount) => amount.to_string(),
        _ if requests == 0 => "0".to_owned(),
        _ => "unknown".to_owned(),
    };
    lines.push(format!(
        "COST: {requests} request(s), {} answered, {} error(s), {} cancelled, {usd} USD",
        count(&filed["responses"]),
        count(&filed["errors"]),
        count(&filed["cancelled"]),
    ));
    lines
}

/// **The live lane's setup, proved offline** (no key, no model, no server):
/// the config it writes — a fixed `[ssh]` stanza like `commander.toml`'s plus
/// [`live_commander_extra`] — loads through the bot's own `Config::load` and
/// validates; its commander resolves from `[arena.commander_model]` with
/// `commander.toml`'s own model name, timeout, reasoning off and provider
/// order; `lead_start_grace_ms` is the posed lanes' 5 s; [`live_lines`]
/// turns a hand-written report of two matches into the three line shapes;
/// and (cut AN.1) [`from_muster`] reads unset and `0` as posed, `1` as the
/// muster, and refuses anything else with its sentence.
#[test]
fn the_live_lane_config_loads_and_resolves() {
    let dir = std::env::temp_dir().join(format!("mush-bot-live-lane-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    let path = dir.join("live.toml");
    let written = format!(
        r#"[ssh]
host = "127.0.0.1"
port = 2222
username = "pilot+Pilot"
password_env = "MUSH_BOT_PASSWORD"
timeout_secs = 15

[agent]
max_seconds = {}

[memory]
persistent = false

[costs]
persistent = false
{}"#,
        arena_rig::MAX_SECONDS,
        live_commander_extra()
    );
    std::fs::write(&path, &written).expect("the config is written");
    let loaded = Config::load(&path);
    let _ = std::fs::remove_dir_all(&dir);
    let config =
        loaded.unwrap_or_else(|error| panic!("the live config loads ({error}):\n{written}"));
    config.validate().expect("the live config validates");

    // What it must match: commander.toml as the bot itself reads it.
    let shipped = Config::load(std::path::Path::new(COMMANDER_TOML)).expect("commander.toml loads");
    let table = shipped
        .arena
        .commander_model
        .clone()
        .expect("commander.toml's [arena.commander_model]");

    assert!(config.arena.commander, "the commander is on");
    assert_eq!(config.arena.lead_start_grace_ms, 5_000);
    assert_eq!(config.arena.commander_ms, shipped.arena.commander_ms);
    assert_eq!(
        config.arena.commander_max_calls,
        shipped.arena.commander_max_calls
    );
    assert!(!config.arena.commander_from_session);
    assert!(config.model.is_none(), "no session model");

    let resolved = commander::resolve(&config.arena, None).expect("a commander resolves");
    assert!(resolved.from_table, "from [arena.commander_model]");
    let model = &resolved.selection.settings;
    assert_eq!(model.name, table.name);
    assert_eq!(model.endpoint, table.endpoint);
    assert_eq!(model.api_key_env.as_deref(), Some(KEY_ENV));
    assert!(table.timeout_secs <= COMMANDER_TIMEOUT_MAX_S);
    assert_eq!(model.timeout_secs, table.timeout_secs);
    let reasoning = model.reasoning.as_ref().expect("reasoning is sent");
    assert_eq!(
        reasoning.get("enabled"),
        Some(&json!(false)),
        "reasoning off"
    );
    assert_eq!(model.reasoning, table.reasoning);
    let provider = model.provider.as_ref().expect("a provider order is sent");
    assert_eq!(
        provider.get("order"),
        table.provider.as_ref().and_then(|p| p.get("order"))
    );
    assert!(provider.get("order").is_some_and(Value::is_array));
    assert_eq!(model.provider, table.provider);

    // The three line shapes, from a hand-written report.
    let report = json!({
        "arena": {
            "wins": 1, "losses": 0, "draws": 0, "timeouts": 1, "other": 0,
            "matches": [
                {"match": 1, "result": "red", "shots": 12, "hits_scored": 9, "kills": 1,
                 "hits_taken": 3, "duration_ms": 187_400, "commander_calls": 80,
                 "commander_applied": 78, "commander_stale": 1, "commander_invalid": 0,
                 "commander_errors": 1, "commander_overruled": 3, "commander_off": null},
                {"match": 2, "result": "timeout", "shots": 20, "hits_scored": 11, "kills": 0,
                 "hits_taken": 6, "duration_ms": 420_000, "commander_calls": 150,
                 "commander_applied": 149, "commander_stale": 0, "commander_invalid": 1,
                 "commander_errors": 0, "commander_overruled": 0,
                 "commander_off": "answers arrive too late to use"}
            ]
        },
        "costs": {"session": {"by_prompt_type": {"arena_commander": {
            "requests": 230, "responses": 229, "errors": 1, "cancelled": 0,
            "provider_cost_by_currency": {"USD": "0.0321"}
        }}}}
    });
    assert_eq!(
        live_lines(&report),
        [
            "LIVE COMMANDER: 1 of 5 for red against one machine at 1 v 1 (lost 0, drew 0, timed out 1, other 0)",
            "match 1: red; 12 shots, 9 hits, took 3; 187 s; commander 80 calls, 78 applied, 1 late, 3 overruled, 0 invalid, 1 errors, off: none",
            "match 2: timeout; 20 shots, 11 hits, took 6; 420 s; commander 150 calls, 149 applied, 0 late, 0 overruled, 1 invalid, 0 errors, off: answers arrive too late to use",
            "COST: 230 request(s), 229 answered, 1 error(s), 0 cancelled, 0.0321 USD",
        ]
    );

    // The muster variable's three readings, and its refusal (cut AN.1).
    assert!(!from_muster(None));
    assert!(!from_muster(Some("0")));
    assert!(from_muster(Some("1")));
    let refused = std::panic::catch_unwind(|| from_muster(Some("yes"))).expect_err("refused");
    assert_eq!(
        refused.downcast_ref::<String>().map(String::as_str),
        Some("MUSH_BOT_LIVE_FROM_MUSTER is 0 or 1, not yes")
    );
}

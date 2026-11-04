//! Doc 43 cut TS.4: arbitrary command lines through the production registry.
//!
//! Every player line runs `dispatch_line` on the world task, and a panic there
//! is caught by `guard()` and **stops the world frozen** (doc 43 §5.1). The
//! property: no line panics, from a staff actor or a plain player, against the
//! engine `boot::build_engine` composes with the TacSim layer on (every verb a
//! server registers, so every handler's argument parsing).
//!
//! Lines are drawn two ways: a registered verb (by name or alias, with a
//! prefix character, switches and hostile argument tokens), and fully
//! arbitrary text. One engine serves every case, so earlier lines' state
//! (objects made, contexts pushed) is in play for later ones; the runner is
//! seeded, so a failure reproduces from the printed line.

use std::cell::RefCell;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mush_config::config::Config;
use mush_core::{GOD, ObjId};
use mush_server::boot;
use mush_server::quarantine::QuarantineSpool;
use mush_store::{InMemoryStore, ObjectStore};
use mush_testkit::TestWorld;
use proptest::prelude::*;
use proptest::test_runner::{Config as RunnerConfig, RngAlgorithm, TestRng, TestRunner};

/// Cases per actor. Each is one dispatch on the booted engine.
const CASES: u32 = 1_500;

fn scratch_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "mush-hostile-lines-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The stock memory-backed config with the compiled-in corridor packs on, as
/// `split_boundary::boot_parity` boots it.
fn config() -> Config {
    let mut cfg = Config::default();
    cfg.store.store = "memory".to_string();
    cfg.server.data_dir = scratch_dir();
    cfg.ipc.listen = "127.0.0.1:0".to_string();
    let dir = cfg.server.data_dir.join("packs");
    std::fs::create_dir_all(&dir).unwrap();
    let core = dir.join("tacsim_core.json");
    let corridor = dir.join("tacsim_corridor.json");
    std::fs::write(&core, tacsim::fixture::CORE_JSON).unwrap();
    std::fs::write(&corridor, tacsim::fixture::CORRIDOR_JSON).unwrap();
    cfg.tacsim.enabled = true;
    cfg.tacsim.pack_paths = vec![core.display().to_string()];
    cfg.tacsim.seed_paths = vec![corridor.display().to_string()];
    cfg.tacsim.corridor = tacsim::fixture::CORRIDOR_PACK.to_string();
    cfg
}

/// Argument tokens that sit on parser edges: object references, numbers past
/// every integer width, the splitters, markup and control characters.
const TOKENS: &[&str] = &[
    "me",
    "here",
    "all",
    "#0",
    "#1",
    "#2",
    "#3",
    "#-1",
    "#99999999999",
    "*God",
    "God",
    "0",
    "-1",
    "-0",
    "1",
    "65535",
    "65536",
    "4294967296",
    "-9223372036854775808",
    "18446744073709551616",
    "1e309",
    "NaN",
    "inf",
    "0x10",
    "1.5",
    "=",
    "==",
    ",",
    ";",
    ":",
    "/",
    "//",
    "\\",
    "&",
    "|",
    "!",
    "(",
    ")",
    "[",
    "]",
    "{",
    "}",
    "%",
    "%r",
    "%0",
    "$",
    "\"",
    "'",
    "`",
    "*",
    "~q",
    "<",
    ">",
    "..",
    "\u{0}",
    "\u{7}",
    "\u{1b}[2J",
    "\r",
    "\t",
    "\u{202e}",
    "\u{feff}",
    "\u{3000}",
    "é",
    "ǅ",
    "👩‍🚀",
    "a",
    "x",
];

/// One hostile argument token: a listed edge, a long run, or anything.
fn token() -> impl Strategy<Value = String> {
    prop_oneof![
        6 => prop::sample::select(TOKENS).prop_map(str::to_owned),
        2 => "[a-z0-9#=,;/ ]{0,12}",
        1 => (1usize..600).prop_map(|n| "a".repeat(n)),
        1 => any::<String>(),
    ]
}

/// A line built on a registered verb.
fn verb_line(words: Vec<String>) -> impl Strategy<Value = String> {
    (
        prop::sample::select(words),
        prop::sample::select(vec!["", "", "", "+", "@", "&", "\"", ":", ";", "\\", "~"]),
        prop::collection::vec("[a-z]{0,8}", 0..3),
        prop::collection::vec(
            (
                token(),
                prop::sample::select(vec![" ", " ", "=", ",", "/", ";", ""]),
            ),
            0..8,
        ),
    )
        .prop_map(|(verb, prefix, switches, args)| {
            let mut line = format!("{prefix}{verb}");
            for sw in switches {
                line.push('/');
                line.push_str(&sw);
            }
            line.push(' ');
            for (tok, sep) in args {
                line.push_str(&tok);
                line.push_str(sep);
            }
            line
        })
}

fn any_line(words: Vec<String>) -> impl Strategy<Value = String> {
    prop_oneof![
        4 => verb_line(words),
        1 => any::<String>(),
        1 => prop::collection::vec(token(), 0..10).prop_map(|t| t.join(" ")),
    ]
}

/// Every word the registry dispatches on: names and aliases.
fn verb_words(tw: &TestWorld) -> Vec<String> {
    let mut words: Vec<String> = tw
        .registry()
        .specs()
        .flat_map(|spec| {
            std::iter::once(spec.name.to_string()).chain(spec.aliases.iter().map(|a| a.to_string()))
        })
        .collect();
    words.sort();
    words.dedup();
    words
}

/// Run `CASES` lines as `actor` (re-created if a line destroyed it), and fail
/// on the first that panics. Returns the slowest lines, for the report, and
/// how many lines reached a handler (answered with something but `Huh?`).
fn fuzz(
    tw: &RefCell<TestWorld>,
    words: &[String],
    actor: ObjId,
    seed: u8,
) -> (Vec<(Duration, String)>, u32) {
    let slowest = RefCell::new(Vec::<(Duration, String)>::new());
    let reached = std::cell::Cell::new(0u32);
    let actor = RefCell::new(actor);
    let mut runner = TestRunner::new_with_rng(
        RunnerConfig {
            cases: CASES,
            failure_persistence: None,
            ..RunnerConfig::default()
        },
        TestRng::from_seed(RngAlgorithm::ChaCha, &[seed; 32]),
    );
    let result = runner.run(&any_line(words.to_vec()), |line| {
        let mut tw = tw.borrow_mut();
        let who = {
            let mut who = actor.borrow_mut();
            if tw.world().get(*who).is_none() {
                let name = format!("Probe{}", tw.world().iter().count());
                *who = tw.spawn_player(&name);
            }
            *who
        };
        let started = Instant::now();
        let said = tw.run(who, &line);
        let took = started.elapsed();
        let mut slow = slowest.borrow_mut();
        slow.push((took, line));
        slow.sort_by_key(|s| std::cmp::Reverse(s.0));
        slow.truncate(5);
        if !said
            .iter()
            .any(|(_, event)| mush_testkit::text_of(event).contains("Huh?"))
        {
            reached.set(reached.get() + 1);
        }
        Ok(())
    });
    if let Err(failure) = result {
        panic!("a command line panicked the world task: {failure}");
    }
    (slowest.into_inner(), reached.get())
}

#[tokio::test]
async fn no_line_panics_the_world_task() {
    let cfg = config();
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    let spool = QuarantineSpool::open(cfg.server.data_dir.join("quarantine")).unwrap();
    let booted = boot::build_engine(&cfg, &store, &spool)
        .await
        .expect("the production engine boots");
    let mut tw = TestWorld::from_engine(booted.engine);
    let player = tw.spawn_player("Probe");
    let words = verb_words(&tw);
    assert!(
        words.len() > 100,
        "the production registry: {} words",
        words.len()
    );
    let tw = RefCell::new(tw);

    let (mut slowest, by_player) = fuzz(&tw, &words, player, 1);
    let (by_god, by_staff) = fuzz(&tw, &words, GOD, 2);
    slowest.extend(by_god);
    eprintln!("reached a handler: {by_player}/{CASES} as a player, {by_staff}/{CASES} as God");
    // Most lines are built on a registered verb: a run where they mostly
    // `Huh?` exercised the parser's front door and no handler.
    assert!(
        by_player > CASES / 4 && by_staff > CASES / 4,
        "too few lines reached a handler"
    );
    slowest.sort_by_key(|s| std::cmp::Reverse(s.0));
    for (took, line) in slowest.iter().take(5) {
        let shown: String = line.chars().take(120).collect();
        eprintln!("slowest: {took:?} {shown:?}");
    }
    drop(tw);
    let _ = std::fs::remove_dir_all(&cfg.server.data_dir);
}

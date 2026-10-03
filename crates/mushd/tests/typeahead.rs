//! **A player who types ahead is served** (16b cut B.5, `B-13`; owner default
//! Q7).
//!
//! Before this cut a line that arrived while another command's commit was in
//! flight was refused at once — `The world is committing another action; please
//! retry in a moment.` — so four lines sent in one burst lost three, and every
//! rig in the tree carried a retry loop a player does not have. The world task
//! now holds such a line in a bounded defer queue (four a connection, 64 in all,
//! 250 ms from its arrival) and serves it when the receipt lands.
//!
//! **This file is the story, not the proof.** Over real SSH, on the production
//! module list, one player sends four lines in ONE write — the first a
//! committing verb (`@desc` sets an attribute and answers `Set.`), so the three
//! behind it meet its commit on any store — and reads the four answers in the
//! order typed with no retry anywhere. Whether the three actually met the commit
//! on a given run depends on how fast the in-memory store answers; the proof
//! that a line behind a held commit is served, that a fifth is refused, that an
//! over-long commit refuses in a player's words and that a barrier waits behind
//! queued lines is in `crates/mush-server/src/world_task.rs`'s held-receipt loop
//! tests, where the test holds the receipt itself.
//!
//! The rig is `sim_lobby.rs`'s, cut down to one pilot: the simulator lobby is
//! where `SIM STATUS` answers with the board. It installs no account oracle,
//! because nothing here claims a slot.
//!
//! **16b cut AP.4 (16c `N20`)** tells the same burst a second time, on a
//! connection that set `OUTPUTPREFIX >>>` and `OUTPUTSUFFIX <<<`: four pairs,
//! each round its own line's answer in the order typed, and a say from another
//! player arriving outside every pair. The proof of the classes (the prefix
//! before the ack, the suffix after it on either outcome, the world task's own
//! refusals bracketed) is in `world_task.rs`'s `ap4_*` tests and the engine's.

mod support;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use mush_core::module::GameModule;
use mush_core::{Engine, GOD, LIMBO, ObjId, ObjKind};
use mush_llm::{LlmProvider, LlmService, MockProvider};
use mush_session::{SessionConfig, SessionManager};
use mush_store::{AccountRec, CharacterRef, InMemoryStore, ObjectStore};
use mushd::accounts::{Accounts, hash_password};
use mushd::ipguard::IpGuard;
use mushd::modules;
use mushd::persist::load_or_seed_world;
use mushd::ssh_bridge::{SshBridge, SshListener, start_listener};
use mushd::staff::{StaffQueue, register_server_commands};
use mushd::world_task::{self, PersistHandles, ServerDeps};

use support::{Term, play_client, scratch_dir, strip_ansi};

const GM_PASSWORD: &str = "gm-secret-1";
const PILOT_PASSWORD: &str = "pilot-secret-1";

/// The phrase both commit refusals end with — the old immediate one and the
/// cut's timed-out one. The story asserts it never appears.
const RETRY: &str = "please retry in a moment";
/// How many times the SETUP (never the story) re-sends a line on that refusal.
const RETRY_BUDGET: usize = 8;

/// The one pilot, created before the world task spawns so its id is stable.
const ADA: ObjId = ObjId(2);

struct Server {
    addr: SocketAddr,
    dir: PathBuf,
    _listener: SshListener,
    _handles: PersistHandles,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The production module list, in production order, with the starter area on:
/// the simulator lobby is part of the atlas-owned OOC pack.
fn module_list(
    onboarding: &mush_core::OnboardingConfig,
    llm: &Arc<LlmService>,
) -> Vec<Box<dyn GameModule>> {
    let mut mods = modules::server_modules();
    mods.push(modules::worldgen_module(Arc::clone(llm)));
    mods.push(modules::npc_module(modules::npc_agent_adapter()));
    mods.extend(modules::sheet_modules(onboarding, &Default::default()));
    mods.push(modules::tacsim_module(tacsim::TacsimConfig {
        enabled: true,
        ..tacsim::TacsimConfig::default()
    }));
    mods.push(modules::content_module_with(true, true).expect("the content module composes"));
    mods
}

async fn boot(tag: &str) -> Server {
    let dir = scratch_dir(tag);
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    store
        .create_account(AccountRec {
            id: None,
            name: "boss".to_string(),
            argon2_hash: hash_password(GM_PASSWORD).expect("a hash"),
            created: 0,
            characters: vec![CharacterRef {
                id: GOD,
                name: "God".to_string(),
            }],
            ssh_keys: vec![],
            discord: None,
        })
        .await
        .expect("the staff account");

    let mut world = load_or_seed_world(&*store).await.expect("boot world");
    let made = world
        .create(ObjKind::Player, "Ada", GOD, Some(LIMBO))
        .expect("a pilot");
    assert_eq!(made, ADA, "pilot id assumption");
    store
        .create_account(AccountRec {
            id: None,
            name: "ada".to_string(),
            argon2_hash: hash_password(PILOT_PASSWORD).expect("a hash"),
            created: i64::from(ADA.0),
            characters: vec![CharacterRef {
                id: ADA,
                name: "Ada".to_string(),
            }],
            ssh_keys: vec![],
            discord: None,
        })
        .await
        .expect("the pilot account");

    let onboarding = mush_core::OnboardingConfig {
        chargen_required: false,
        start_room: LIMBO,
        start_room_unapproved: LIMBO,
    };
    let staff = Arc::new(StaffQueue::new());
    let mut registry = world_task::base_registry();
    register_server_commands(&mut registry, &staff).expect("server commands");
    let mock: Arc<dyn LlmProvider> = Arc::new(MockProvider::new("openrouter"));
    let llm = Arc::new(
        LlmService::builder()
            .provider("openrouter", mock)
            .global_enabled(false)
            .build(),
    );
    let mut engine =
        Engine::with_modules(world, registry, module_list(&onboarding, &llm)).expect("modules");
    modules::boot_modules(&mut engine, &*store)
        .await
        .expect("module boot");

    let sessions = Arc::new(SessionManager::with_config(SessionConfig {
        linkdead_grace: Duration::from_secs(120),
        ..SessionConfig::default()
    }));
    let guard = Arc::new(IpGuard::new());
    let handles = world_task::spawn_persist(
        engine,
        Arc::clone(&sessions),
        ServerDeps {
            store: Arc::clone(&store),
            guard: Arc::clone(&guard),
            staff,
            llm: Some(Arc::clone(&llm)),
            quarantine_spool: mushd::quarantine::QuarantineSpool::in_memory(),
            metrics: Arc::new(mush_net_web::Metrics::new()),
            outbox_adapters: Vec::new(),
        },
    );
    let accounts = Arc::new(Accounts::new(Arc::clone(&store), handles.handle.clone(), 6));
    let bridge = Arc::new(SshBridge::new(
        accounts,
        Arc::clone(&sessions),
        handles.handle.clone(),
        Arc::clone(&guard),
    ));
    let listener = start_listener(
        "127.0.0.1:0".parse().expect("a loopback address"),
        &dir,
        "Welcome to the simulator.".to_string(),
        bridge,
    )
    .await
    .expect("ssh listener");
    let addr = listener.addr();
    Server {
        addr,
        dir,
        _listener: listener,
        _handles: handles,
    }
}

/// Connect one account and wait for its character to land.
async fn login(
    addr: SocketAddr,
    account: &str,
    password: &str,
    character: &str,
) -> (russh::client::Handle<support::AcceptAll>, Term) {
    let (handle, mut term) = play_client(addr).await;
    term.expect("Welcome to the simulator.").await;
    term.send_line(&format!("connect {account} {password}"))
        .await;
    term.expect(&format!("Connected as {character}")).await;
    (handle, term)
}

/// SETUP ONLY: run one line and wait for `needle`, re-sending on a commit
/// refusal (the staff account's teleport can meet the first-boot pack apply).
async fn say(term: &mut Term, line: &str, needle: &str) {
    for _ in 0..RETRY_BUDGET {
        let mark = term.mark();
        term.send_line(line).await;
        if term.expect_one_of_after(mark, &[needle, RETRY]).await == 0 {
            return;
        }
    }
    panic!("`{line}` was refused as commit-pending {RETRY_BUDGET} times running");
}

/// One object's dbref, out of an `@find` listing.
fn dbref_of(listing: &str, name: &str) -> u32 {
    listing
        .lines()
        .find_map(|line| {
            let rest = line.split_once(name)?.1;
            let inside = rest.split_once("(#")?.1;
            inside.split(')').next()?.trim().parse::<u32>().ok()
        })
        .unwrap_or_else(|| panic!("no dbref for {name:?} in:\n{listing}"))
}

/// The byte offset of `needle` in `text` at or after `from`.
fn find_from(text: &str, from: usize, needle: &str) -> usize {
    text.get(from..)
        .and_then(|rest| rest.find(needle))
        .map(|at| from + at)
        .unwrap_or_else(|| panic!("{needle:?} not found after byte {from} in:\n{text}"))
}

/// **Four lines in one write, four answers in order, no retry.**
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn four_lines_typed_ahead_behind_a_commit_are_all_answered_in_order() {
    let server = boot("typeahead").await;

    // Setup: staff walk Ada into the simulator lobby, where `SIM STATUS` reads
    // the board.
    let (_god_handle, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    let mark = god.mark();
    say(&mut god, "@find Simulator Lobby", "END OF LIST").await;
    let lobby = dbref_of(&strip_ansi(&god.seen[mark..]), "Simulator Lobby");
    say(
        &mut god,
        &format!("@teleport #{}=#{lobby}", ADA.0),
        "Teleported",
    )
    .await;

    let (_ada_handle, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    // Let the attach look land before the mark, so everything after it is the
    // answer to the burst.
    ada.expect("Simulator Lobby").await;

    // The story: ONE write carrying four lines. The first commits (an
    // attribute set), so the three behind it meet its commit.
    let mark = ada.mark();
    ada.send_line("@desc me=Typing ahead.\rlook\rsim status\rwho")
        .await;
    // The WHO header's last column is the last of the four answers to start.
    ada.expect_after(mark, "Doing").await;
    // And wait for the whole WHO to finish (its footer), so a late refusal of
    // any line would have had its chance to arrive.
    ada.expect_after(mark, "in your room.").await;

    let heard = strip_ansi(&ada.seen[mark..]);
    assert!(
        !heard.contains(RETRY),
        "no line of the burst was refused: {heard}"
    );
    let set = find_from(&heard, 0, "Set.");
    let room = find_from(&heard, set, "Simulator Lobby");
    let board = find_from(&heard, room, "Simulator centre:");
    let who = find_from(&heard, board, "Doing");
    assert!(
        set < room && room < board && board < who,
        "`Set.`, the room, the board and the WHO header, in the order typed: {heard}"
    );
}

/// The lines of `text`, each trimmed, the empty ones dropped — what a script
/// reading a line-mode connection sees, one line at a time.
fn script_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Read until `term` has shown at least `n` lines that are exactly `<<<`
/// since byte `from` (the server echoes typed lines, so a line that merely
/// contains `<<<`, such as the echo of `OUTPUTSUFFIX <<<`, does not count).
async fn read_suffixes(term: &mut Term, from: usize, n: usize) {
    loop {
        let seen = script_lines(&strip_ansi(&term.seen[from..]))
            .iter()
            .filter(|line| *line == "<<<")
            .count();
        if seen >= n {
            return;
        }
        let mark = term.mark();
        term.expect_after(mark, "<<<").await;
    }
}

/// **16b cut AP.4 (16c `N20`): the brackets, over real SSH on the production
/// module list.** Ada sets `OUTPUTPREFIX >>>` (silent) and `OUTPUTSUFFIX <<<`
/// (its own line reads `>>>` then `<<<`: the prefix stood, the suffix is read
/// after the body), then sends the four lines above in ONE write with the
/// committing `@desc` first. She reads four pairs in the order typed, each
/// round that line's own answer and nothing else, with no retry; then a say
/// from God in her room arrives OUTSIDE every pair. The brackets are her own
/// text echoed back and name nothing, so the observer rule has nothing to
/// lens here; the typed-ahead burst and the notice rule are what this proves.
/// Whether the three lines behind `@desc` meet its commit depends on how fast
/// the in-memory store answers (the file head), so the held case is proved
/// deterministically, on a receipt the test holds, by `mush-server`'s
/// `ap4_a_line_held_behind_a_save_reads_its_own_pair_after_the_saving_one`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn brackets_round_four_lines_typed_ahead_in_one_write_and_round_no_notice() {
    let server = boot("typeahead-brackets").await;

    // Setup: staff walk Ada into the simulator lobby and go there too, so a
    // say of God's reaches her.
    let (_god_handle, mut god) = login(server.addr, "boss", GM_PASSWORD, "God").await;
    let mark = god.mark();
    say(&mut god, "@find Simulator Lobby", "END OF LIST").await;
    let lobby = dbref_of(&strip_ansi(&god.seen[mark..]), "Simulator Lobby");
    say(
        &mut god,
        &format!("@teleport #{}=#{lobby}", ADA.0),
        "Teleported",
    )
    .await;
    say(
        &mut god,
        &format!("@teleport me=#{lobby}"),
        "Simulator Lobby",
    )
    .await;

    let (_ada_handle, mut ada) = login(server.addr, "ada", PILOT_PASSWORD, "Ada").await;
    ada.expect("Simulator Lobby").await;

    // The two settings. The prefix line is silent; the suffix line is read
    // with the prefix in front of it and its own suffix after it.
    let mark = ada.mark();
    ada.send_line("OUTPUTPREFIX >>>").await;
    ada.send_line("OUTPUTSUFFIX <<<").await;
    read_suffixes(&mut ada, mark, 1).await;
    let settings = script_lines(&strip_ansi(&ada.seen[mark..]));
    let framed: Vec<&String> = settings
        .iter()
        .filter(|line| *line == ">>>" || *line == "<<<")
        .collect();
    assert_eq!(
        framed,
        [">>>", "<<<"],
        "OUTPUTPREFIX said nothing; OUTPUTSUFFIX read its pair: {settings:?}"
    );
    assert!(
        settings
            .iter()
            .all(|line| line == ">>>" || line == "<<<" || line.starts_with("OUTPUT")),
        "nothing but the pair (and any echo of the typed lines): {settings:?}"
    );

    // The story: ONE write carrying four lines, the first committing.
    let burst = ada.mark();
    ada.send_line("@desc me=Typing ahead.\rlook\rsim status\rwho")
        .await;
    // The WHO table's footer is inside the fourth pair; read to its suffix.
    read_suffixes(&mut ada, burst, 4).await;
    let heard = strip_ansi(&ada.seen[burst..]);
    assert!(
        !heard.contains(RETRY),
        "no line of the burst was refused: {heard}"
    );
    // Walk the pairs: every `>>>` opens one, the next `<<<` closes it, and
    // a pair never opens inside another.
    // The server echoes what was typed; a script skips its own lines.
    let typed = ["@desc me=Typing ahead.", "look", "sim status", "who"];
    let lines: Vec<String> = script_lines(&heard)
        .into_iter()
        .filter(|line| !typed.iter().any(|t| line.ends_with(t)))
        .collect();
    let mut pairs: Vec<Vec<String>> = Vec::new();
    let mut open: Option<Vec<String>> = None;
    for line in &lines {
        match (line.as_str(), open.as_mut()) {
            (">>>", None) => open = Some(Vec::new()),
            (">>>", Some(_)) => panic!("a pair opened inside another: {lines:?}"),
            ("<<<", Some(_)) => pairs.extend(open.take()),
            ("<<<", None) => panic!("a suffix with no prefix: {lines:?}"),
            (_, Some(inside)) => inside.push(line.clone()),
            (_, None) => {}
        }
    }
    assert!(open.is_none(), "every pair closed: {lines:?}");
    assert_eq!(pairs.len(), 4, "four pairs, one a line: {lines:?}");
    // Each pair holds that line's own answer, in the order typed.
    assert_eq!(
        pairs[0],
        ["Set."],
        "the first pair is `@desc`'s `Set.` alone"
    );
    assert!(
        pairs[1]
            .first()
            .is_some_and(|line| line.contains("Simulator Lobby")),
        "the second pair is the room: {pairs:?}"
    );
    assert!(
        pairs[2]
            .iter()
            .any(|line| line.contains("Simulator centre:")),
        "the third pair is the board: {pairs:?}"
    );
    assert!(
        pairs[3].iter().any(|line| line.contains("Doing"))
            && pairs[3].iter().any(|line| line.contains("in your room.")),
        "the fourth pair is the WHO table, header to footer: {pairs:?}"
    );
    // No answer leaks into another line's pair.
    for (n, needle) in ["Set.", "Simulator centre:", "in your room."]
        .into_iter()
        .enumerate()
    {
        let owner = [0, 2, 3][n];
        for (at, pair) in pairs.iter().enumerate() {
            if at != owner {
                assert!(
                    !pair.iter().any(|line| line.contains(needle)),
                    "{needle:?} sits in pair {at}, not {owner}: {pairs:?}"
                );
            }
        }
    }

    // A notice: God says a line in Ada's room. It is pushed, not an answer,
    // so it arrives outside every pair.
    let notice = ada.mark();
    say(
        &mut god,
        "say The tower is listening.",
        "The tower is listening.",
    )
    .await;
    ada.expect_after(notice, "The tower is listening.").await;
    let all = script_lines(&strip_ansi(&ada.seen[mark..]));
    let count = |needle: &str| all.iter().filter(|line| *line == needle).count();
    assert_eq!(
        (count(">>>"), count("<<<")),
        (5, 5),
        "one pair for OUTPUTSUFFIX's own line and four for the burst; the say adds none: {all:?}"
    );
    let said = all
        .iter()
        .position(|line| line.contains("The tower is listening."))
        .expect("the say reached Ada");
    assert!(
        said == 0 || all[said - 1] != ">>>",
        "the say is not opened by a prefix: {all:?}"
    );
    assert!(
        all.get(said + 1).is_none_or(|line| line != "<<<"),
        "the say is not closed by a suffix: {all:?}"
    );
    let opened = all[..said].iter().filter(|line| *line == ">>>").count();
    let closed = all[..said].iter().filter(|line| *line == "<<<").count();
    assert_eq!(
        opened, closed,
        "the say lands between pairs, not in one: {all:?}"
    );
}

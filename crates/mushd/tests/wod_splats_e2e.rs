//! M6b oWoD20 **splats** end-to-end over real SSH (in-memory store, ungated):
//! the same production wiring as `wod_e2e.rs` (world task, session layer,
//! accounts, SSH listener, built-ins + the seeded WoD ruleset) driving **three**
//! characters — one Vampire (V20), one Mage (M20), one Werewolf (W20) — each
//! through the per-template dispatch introduced in M6b:
//!
//! 1. every character runs `+chargen`, picks their **template** with
//!    `+chargen/template`, allocates their splat traits, and is staff-approved;
//! 2. `+sheet` renders **only** that splat's sections (a vampire has Disciplines
//!    but no Spheres / Garou block; a mage has Spheres but no Disciplines; a
//!    Garou has the Garou block but neither) — the dispatch acceptance test;
//! 3. the Vampire `+disc/roll`s a Discipline pool and `+blood/spend`s;
//! 4. the Mage `+cast`s (rolling Arete vs a difficulty) and a Sphere is
//!    refused above the caster's Arete;
//! 5. the Werewolf `+shift`s to Crinos and a subsequent Strength `+roll`
//!    reflects the war form's +4, and `+rage/spend`s;
//! 6. **cross-splat refusal**: a Mage typing `+blood` is told they are not
//!    Kindred (and symmetric refusals for the other two).
//!
//! The ruleset is seeded so the run is deterministic; dice assertions are
//! structural (the pool's dice count is fully determined by traits + form,
//! independent of how many draws earlier commands consumed) and the rendered
//! ANSI is asserted the way the clients actually read it.

mod support;

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use mush_core::{Engine, GOD, World};
use mush_session::{SessionConfig, SessionManager};
use mush_store::{AccountRec, CharacterRef, InMemoryStore, ObjectStore};

use mushd::accounts::{Accounts, hash_password};
use mushd::ipguard::IpGuard;
use mushd::ssh_bridge::{SshBridge, start_listener};
use mushd::staff::{StaffQueue, register_server_commands};
use mushd::world_task::{self, ServerDeps};
use mushd::{modules, unix_now_s};

use support::{Term, play_client, scratch_dir, strip_ansi};

/// Fixed RNG seed → deterministic dice for the whole run.
const S: u64 = 12345;
const GM_PASSWORD: &str = "gm-secret-splats";

// ---------------------------------------------------------------------
// Boot: production wiring + built-in modules + the seeded WoD ruleset,
// on an ephemeral port (mirrors wod_e2e.rs / main.rs boot order).
// ---------------------------------------------------------------------

async fn boot_server(dir: &Path) -> SocketAddr {
    std::fs::create_dir_all(dir).unwrap();
    // A generous flood budget: three back-to-back chargen bursts (~28 +stat
    // lines each) exercise the ruleset, not the throttle.
    let sessions = Arc::new(SessionManager::with_config(SessionConfig {
        linkdead_grace: Duration::from_secs(120),
        flood_burst: 4000.0,
        ..SessionConfig::default()
    }));
    let store: Arc<dyn ObjectStore> = Arc::new(InMemoryStore::new());
    store
        .create_account(AccountRec {
            name: "boss".to_string(),
            argon2_hash: hash_password(GM_PASSWORD).unwrap(),
            created: 0,
            characters: vec![CharacterRef {
                id: GOD,
                name: "God".to_string(),
            }],
        })
        .await
        .unwrap();

    let staff = Arc::new(StaffQueue::new());
    let mut registry = world_task::base_registry();
    register_server_commands(&mut registry, &staff).unwrap();
    let help_dir = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/help"));
    let _ = mushd::help::load_help_topics(&mut registry, help_dir);

    let mut world = World::new_minimal();
    world.set_now(unix_now_s());
    let mut mods = modules::server_modules();
    mods.push(Box::new(ruleset_wod::WodRuleset::with_seed(S)));
    let mut engine = Engine::with_modules(world, registry, mods).unwrap();
    modules::boot_modules(&mut engine, &*store).await.unwrap();

    let handles = world_task::spawn_persist(
        engine,
        Arc::clone(&sessions),
        ServerDeps {
            store: Arc::clone(&store),
            guard: Arc::new(IpGuard::new()),
            staff,
        },
    );
    let world = handles.handle;
    let accounts = Arc::new(Accounts::new(Arc::clone(&store), world.clone(), 3));
    let bridge = Arc::new(SshBridge::new(
        accounts,
        Arc::clone(&sessions),
        world,
        Arc::new(IpGuard::new()),
    ));
    start_listener(
        "127.0.0.1:0".parse().unwrap(),
        dir,
        "Welcome to the splat e2e MUSH!".to_string(),
        bridge,
    )
    .await
    .expect("listener")
    .addr()
}

async fn login_create(
    addr: SocketAddr,
    name: &str,
    pw: &str,
) -> (russh::client::Handle<support::AcceptAll>, Term) {
    let (handle, mut term) = play_client(addr).await;
    term.expect("Welcome to the splat e2e MUSH!").await;
    term.send_line(&format!("create {name} {pw}")).await;
    term.expect(&format!("Connected as {name}")).await;
    term.expect("Limbo(#0)").await;
    (handle, term)
}

/// Send a burst of lines back-to-back (per-connection ordering guarantees they
/// all land before the next command this connection issues).
async fn send_all(term: &mut Term, lines: &[&str]) {
    for l in lines {
        term.send_line(l).await;
    }
}

// ---------------------------------------------------------------------
// Chargen scripts (the balanced sequences proven in
// ruleset-wod/tests/wod.rs, driven here over the SSH front door).
// ---------------------------------------------------------------------

/// The shared, budget-balancing base allocation (attributes / abilities /
/// backgrounds) used by all three splats after the priorities are set.
const BASE_ALLOC: &[&str] = &[
    "+stat strength=4",
    "+stat dexterity=4",
    "+stat stamina=2",
    "+stat charisma=3",
    "+stat manipulation=3",
    "+stat appearance=2",
    "+stat perception=2",
    "+stat intelligence=2",
    "+stat wits=2",
    "+stat alertness=3",
    "+stat athletics=3",
    "+stat brawl=3",
    "+stat empathy=2",
    "+stat expression=2",
    "+stat drive=3",
    "+stat firearms=3",
    "+stat stealth=3",
    "+stat academics=3",
    "+stat computer=2",
];

const PRIORITIES: &[&str] = &[
    "+chargen/attr physical=7 social=5 mental=3",
    "+chargen/abil talents=13 skills=9 knowledges=5",
];

/// Run one character all the way from `+chargen` through staff approval, using
/// the given splat-specific setup lines and the final +stat/+sphere marker to
/// fence on. `job` is the wod-chargen job id this submit will open.
async fn chargen_and_approve(
    player: &mut Term,
    god: &mut Term,
    template: &str,
    pre_base: &[&str],
    post_base: &[&str],
    final_marker: &str,
    job: u64,
) {
    player.send_line("+chargen").await;
    player.expect("Chargen started.").await;
    send_all(player, &[&format!("+chargen/template {template}")]).await;
    send_all(player, PRIORITIES).await;
    send_all(player, pre_base).await;
    send_all(player, BASE_ALLOC).await;
    send_all(player, post_base).await;
    player.expect(final_marker).await;

    let mark = player.mark();
    player.send_line("+chargen/status").await;
    player.expect_after(mark, "Budget balances").await;

    player.send_line("+chargen/submit").await;
    player
        .expect(&format!("Submitted for approval as job {job}"))
        .await;

    let gmark = god.mark();
    god.send_line(&format!("+job/approve {job}=Welcome.")).await;
    god.expect_after(gmark, &format!("Job {job} approved."))
        .await;
    player.expect("has been approved by God").await;
}

/// Render `+sheet`, fence on a trailing `+xp` (universal, distinctive output)
/// so the whole sheet has certainly arrived, and return its ANSI-stripped text.
async fn sheet_text(player: &mut Term) -> String {
    let mark = player.mark();
    player.send_line("+sheet").await;
    player.send_line("+xp").await;
    player.expect_after(mark, "XP available").await;
    strip_ansi(&player.seen[mark..])
}

// ---------------------------------------------------------------------
// The journey
// ---------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn m6b_wod_splats_full_journey() {
    let dir = scratch_dir("wod-splats-e2e");
    let addr = boot_server(&dir).await;

    // --- God + the three splat characters. -------------------------------
    let (_god_handle, mut god) = play_client(addr).await;
    god.expect("Welcome to the splat e2e MUSH!").await;
    god.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    god.expect("Connected as God").await;
    god.expect("Limbo(#0)").await;

    let (_vlad_handle, mut vlad) = login_create(addr, "Vlad", "secret123").await;
    god.expect("Vlad has connected.").await;
    let (_mira_handle, mut mira) = login_create(addr, "Mira", "secret123").await;
    god.expect("Mira has connected.").await;
    let (_fenris_handle, mut fenris) = login_create(addr, "Fenris", "secret123").await;
    god.expect("Fenris has connected.").await;

    // ================= Vampire (Brujah) chargen → job 1 =================
    chargen_and_approve(
        &mut vlad,
        &mut god,
        "vampire",
        &["+stat clan=brujah"],
        &[
            "+stat backgrounds:Resources=3",
            "+stat backgrounds:Contacts=2",
            "+stat disciplines:Celerity=2",
            "+stat disciplines:Potence=1",
            "+stat conscience=3",
            "+stat self_control=3",
            "+stat courage=4",
        ],
        "Set courage = 4.",
        1,
    )
    .await;

    // ================= Mage (Order of Hermes) chargen → job 2 =================
    chargen_and_approve(
        &mut mira,
        &mut god,
        "mage",
        &["+stat affiliation=order_of_hermes"],
        &[
            "+stat backgrounds:Avatar=2",
            "+stat backgrounds:Resources=3",
            // Arete 1 → 3 (2 dots over the free 1st, 8 freebies) so +cast rolls
            // a meaty pool and the Sphere cap sits at 3.
            "+stat arete=3",
            "+sphere Forces=1",
            "+sphere Correspondence=1",
            "+sphere Life=1",
            "+sphere Mind=1",
            "+sphere Prime=1",
        ],
        "Prime set to 1",
        2,
    )
    .await;

    // ================= Werewolf (Lupus/Ahroun/Silver Fangs) chargen → job 3 ==
    chargen_and_approve(
        &mut fenris,
        &mut god,
        "werewolf",
        &[
            "+stat breed=lupus",
            "+stat auspice=ahroun",
            "+stat tribe=silver_fangs",
        ],
        &[
            "+stat backgrounds:Resources=3",
            "+stat backgrounds:Contacts=2",
        ],
        "Set backgrounds:Contacts = 2.",
        3,
    )
    .await;

    // ================= +sheet dispatch: each carries ONLY its splat ==========
    let vsheet = sheet_text(&mut vlad).await;
    assert!(vsheet.contains("DISCIPLINES"), "vampire sheet: {vsheet}");
    assert!(vsheet.contains("Blood Pool"), "vampire Blood Pool");
    assert!(!vsheet.contains("SPHERES"), "a vampire is not a mage");
    assert!(!vsheet.contains("GAROU"), "a vampire is not Garou");

    let msheet = sheet_text(&mut mira).await;
    assert!(msheet.contains("SPHERES"), "mage sheet: {msheet}");
    assert!(msheet.contains("Arete"), "mage Arete");
    assert!(!msheet.contains("DISCIPLINES"), "a mage is not Kindred");
    assert!(!msheet.contains("GAROU"), "a mage is not Garou");

    let wsheet = sheet_text(&mut fenris).await;
    assert!(wsheet.contains("GAROU"), "werewolf sheet: {wsheet}");
    assert!(wsheet.contains("Rage"), "werewolf Rage");
    assert!(!wsheet.contains("DISCIPLINES"), "a Garou is not Kindred");
    assert!(!wsheet.contains("SPHERES"), "a Garou is not a mage");

    // ================= Vampire: +disc/roll + +blood/spend ====================
    // Celerity 2 + Dexterity 4 = 6 dice at difficulty 6.
    let mark = vlad.mark();
    vlad.send_line("+disc/roll celerity+dexterity vs 6").await;
    vlad.expect_after(mark, "(celerity+dexterity, 6 dice, diff 6)")
        .await;
    // 13th-generation Blood Pool starts full at 10; spend 3 → 7.
    let mark = vlad.mark();
    vlad.send_line("+blood/spend 3").await;
    vlad.expect_after(mark, "Blood 7/10").await;

    // ================= Mage: +cast (Arete pool) + Sphere Arete-cap ===========
    // Arete 3 → 3 dice at difficulty 7.
    let mark = mira.mark();
    mira.send_line("+cast Hurl a bolt of fire vs 7").await;
    mira.expect_after(mark, "(arete, 3 dice, diff 7)").await;
    // A Sphere above Arete (3) is refused.
    let mark = mira.mark();
    mira.send_line("+sphere Forces=4").await;
    mira.expect_after(mark, "cannot exceed your Arete").await;

    // ================= Werewolf: +shift Crinos reaches +roll, +rage/spend ====
    let mark = fenris.mark();
    fenris.send_line("+shift crinos").await;
    fenris.expect_after(mark, "crinos").await;
    fenris.expect_after(mark, "Delirium").await;
    // Strength 4 + Crinos +4 = 8 dice reaching the base +roll.
    let mark = fenris.mark();
    fenris.send_line("+roll strength").await;
    fenris.expect_after(mark, "(8 dice, diff 6)").await;
    // Ahroun Rage 5 (permanent); spend 2 from the temporary pool → 3/5.
    let mark = fenris.mark();
    fenris.send_line("+rage/spend 2").await;
    fenris.expect_after(mark, "Rage 3/5").await;

    // ================= Cross-splat refusal (self-gating commands) ============
    // The headline case: a Mage typing +blood is told they are not Kindred.
    let mark = mira.mark();
    mira.send_line("+blood").await;
    mira.expect_after(mark, "not Kindred").await;
    // And symmetric refusals both other ways.
    let mark = vlad.mark();
    vlad.send_line("+cast Something").await;
    vlad.expect_after(mark, "not Awakened").await;
    let mark = fenris.mark();
    fenris.send_line("+disc").await;
    fenris.expect_after(mark, "not Kindred").await;

    // ================= raw stream really was ANSI ===========================
    assert!(
        vlad.seen.contains('\u{1b}'),
        "WoD SSH output must carry ANSI escapes"
    );
    assert!(strip_ansi(&fenris.seen).contains("Fenris rolls strength (8 dice, diff 6)"));

    let _ = std::fs::remove_dir_all(&dir);
}

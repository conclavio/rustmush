//! M6a oWoD20 end-to-end journey over real SSH (in-memory store, ungated):
//! the full server wiring — world task, session layer, accounts, SSH
//! listener — booted exactly as `main.rs` does (`Engine::with_modules` →
//! `boot_modules`) with the **built-in modules plus the WoD ruleset** (the
//! `[rulesets].enabled = ["wod"]` selection), then a player driving the whole
//! ruleset surface end to end and a staff character approving their jobs —
//! all asserted on the rendered ANSI the clients actually read:
//!
//! 1. a fresh player runs `+chargen` (priority point-buy) and `+chargen/submit`;
//! 2. God (`+job/approve`) approves the `wod-chargen` job → the sheet is Approved;
//! 3. the approved character `+roll`s `strength+brawl vs 6` and the room (a
//!    witness) sees a correct classic Storyteller result line;
//! 4. an authoritative roll spends temporary Willpower (a second is unavailable);
//! 5. `+harm` (staff) marks Lethal damage and the wound penalty then shrinks a
//!    subsequent pool;
//! 6. `+xp/award` + `+xp/spend` files a costed raise; on `+job/approve` the stat
//!    goes up and the XP is deducted;
//! 7. `+sheet` renders the classic dotted sheet.
//!
//! The ruleset is built with a fixed RNG seed so the run is deterministic, but
//! the dice assertions are structural (dice count, wound tag, result line
//! present) rather than pinned to exact successes — robust regardless of how
//! many draws earlier commands consumed.

mod support;

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use mush_core::{
    Engine, GOD, OfferKeyRing, OfferSecret, ResolutionKeyRing, ResolutionSecret, World,
};
use mush_session::{SessionConfig, SessionManager};
use mush_store::{AccountRec, CharacterRef, InMemoryStore, ObjectStore};

use mushd::accounts::{Accounts, hash_password};
use mushd::ipguard::IpGuard;
use mushd::ssh_bridge::{SshBridge, start_listener};
use mushd::staff::{StaffQueue, register_server_commands};
use mushd::world_task::{self, ServerDeps};
use mushd::{modules, unix_now_s};

use support::{Term, play_client, scratch_dir, strip_ansi, submit_action};

/// Fixed RNG seed → deterministic dice for the whole run.
const S: u64 = 12345;
const GM_PASSWORD: &str = "gm-secret-6";

// ---------------------------------------------------------------------
// Boot: production wiring + built-in modules + the seeded WoD ruleset,
// on an ephemeral port. Mirrors main.rs' boot order exactly, with the
// ruleset appended AFTER the built-ins (as ruleset_modules is, in prod).
// ---------------------------------------------------------------------

async fn boot_server(dir: &Path) -> SocketAddr {
    std::fs::create_dir_all(dir).unwrap();
    // A generous flood budget: this journey fires a long chargen burst
    // (~24 +stat lines) on one connection, and it exercises the ruleset, not
    // the throttle — the default burst of 30 would trip mid-journey.
    let sessions = Arc::new(SessionManager::with_config(SessionConfig {
        linkdead_grace: Duration::from_secs(120),
        flood_burst: 1000.0,
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
            ssh_keys: vec![],
        })
        .await
        .unwrap();

    let staff = Arc::new(StaffQueue::new());
    let mut registry = world_task::base_registry();
    register_server_commands(&mut registry, &staff).unwrap();
    let help_dir = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/help"));
    let topics = mushd::help::load_help_topics(&mut registry, help_dir).unwrap();
    assert!(topics >= 10, "repo help topics must load (got {topics})");

    let mut world = World::new_minimal();
    world.set_now(unix_now_s());
    // Built-ins then the WoD ruleset (seeded), exactly as main.rs composes
    // server_modules() ++ ruleset_modules(), but with a deterministic RNG.
    let mut mods = modules::server_modules();
    mods.push(Box::new(ruleset_wod::WodRuleset::with_seed(S)));
    let mut engine = Engine::with_modules(world, registry, mods).unwrap();
    engine.set_offer_keys(OfferKeyRing::new(OfferSecret::new(1, [0x51; 32]).unwrap(), []).unwrap());
    engine.set_resolution_keys(
        ResolutionKeyRing::new(ResolutionSecret::new(1, [0x52; 32]).unwrap(), []).unwrap(),
    );
    modules::boot_modules(&mut engine, &*store).await.unwrap();

    let handles = world_task::spawn_persist(
        engine,
        Arc::clone(&sessions),
        ServerDeps {
            metrics: std::sync::Arc::new(mush_net_web::Metrics::new()),
            store: Arc::clone(&store),
            guard: Arc::new(IpGuard::new()),
            staff,
            llm: None,
            quarantine_spool: mushd::quarantine::QuarantineSpool::in_memory(),
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
        "Welcome to the WoD e2e MUSH!".to_string(),
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
    term.expect("Welcome to the WoD e2e MUSH!").await;
    term.send_line(&format!("create {name} {pw}")).await;
    term.expect(&format!("Connected as {name}")).await;
    term.expect("Limbo(#0)").await;
    (handle, term)
}

// ---------------------------------------------------------------------
// The journey
// ---------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn m6_wod_full_journey() {
    let dir = scratch_dir("wod-e2e");
    let addr = boot_server(&dir).await;

    // --- God + the player + a roommate witness. --------------------------
    let (_god_handle, mut god) = play_client(addr).await;
    god.expect("Welcome to the WoD e2e MUSH!").await;
    god.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    god.expect("Connected as God").await;
    god.expect("Limbo(#0)").await;

    let (_nick_handle, mut nick) = login_create(addr, "Nick", "secret123").await;
    god.expect("Nick has connected.").await;
    // A witness in the same room, to prove the room sees a public roll.
    let (_bob_handle, mut bob) = login_create(addr, "Bob", "hunter42").await;
    nick.expect("Bob has connected.").await;

    // ================= chargen (priority point-buy) =================
    nick.send_line("+chargen").await;
    nick.expect("Chargen started.").await;
    let mark = nick.mark();
    nick.send_line("+chargen/attr physical=7 social=5 mental=3")
        .await;
    nick.expect_after(mark, "Priorities set.").await;
    let mark = nick.mark();
    nick.send_line("+chargen/abil talents=13 skills=9 knowledges=5")
        .await;
    nick.expect_after(mark, "Priorities set.").await;

    // Attributes (physical 7 = 3+3+1, social 5 = 2+2+1, mental 3 = 1+1+1).
    for c in [
        "+stat strength=4",
        "+stat dexterity=4",
        "+stat stamina=2",
        "+stat charisma=3",
        "+stat manipulation=3",
        "+stat appearance=2",
        "+stat perception=2",
        "+stat intelligence=2",
        "+stat wits=2",
        // Talents 13.
        "+stat alertness=3",
        "+stat athletics=3",
        "+stat brawl=3",
        "+stat empathy=2",
        "+stat expression=2",
        // Skills 9.
        "+stat drive=3",
        "+stat firearms=3",
        "+stat stealth=3",
        // Knowledges 5.
        "+stat academics=3",
        "+stat computer=2",
        // Backgrounds 5.
        "+stat backgrounds:Resources=3",
        "+stat backgrounds:Contacts=2",
    ] {
        let mark = nick.mark();
        nick.send_line(c).await;
        let assignment = c.strip_prefix("+stat ").expect("a +stat command");
        let (field, value) = assignment.split_once('=').expect("field=value");
        nick.expect_after(mark, &format!("Set {field} = {value}."))
            .await;
    }

    let mark = nick.mark();
    nick.send_line("+chargen/status").await;
    nick.expect_after(mark, "Budget balances").await;

    nick.send_line("+chargen/submit").await;
    nick.expect("Submitted for approval as job 1").await;

    // ================= staff approval → sheet Approved =================
    let mark = god.mark();
    god.send_line("+job 1").await;
    god.expect_after(mark, "Chargen approval: Nick").await;
    god.send_line("+job/approve 1=Welcome to the Chronicle.")
        .await;
    god.expect("Job 1 approved.").await;
    nick.expect("has been approved by God").await;

    // The payload handler stamped the sheet Approved: re-running +chargen
    // now refuses because the character is approved.
    let mark = nick.mark();
    nick.send_line("+chargen").await;
    nick.expect_after(mark, "already approved").await;

    // ================= public roll: the room sees a classic result =======
    // strength 4 + brawl 3 = 7 dice, difficulty 6.
    let bob_mark = bob.mark();
    nick.send_line("+roll strength+brawl vs 6").await;
    nick.expect("Nick rolls strength+brawl (7 dice, diff 6) and")
        .await;
    // The witness in the room sees the same public roll.
    bob.expect_after(bob_mark, "Nick rolls strength+brawl (7 dice, diff 6) and")
        .await;

    // ================= willpower spend =================
    // Chargen finalize set temporary Willpower to full (== Courage baseline,
    // 1). One authoritative roll spends it; its offer then disappears.
    submit_action(
        &mut nick,
        r#"+act willpower-roll with {"pool":"strength+brawl","difficulty":6}"#,
        "+WP",
    )
    .await;
    let mark = nick.mark();
    nick.send_line(r#"+act willpower-roll with {"pool":"strength+brawl","difficulty":6}"#)
        .await;
    nick.expect_after(mark, "That action is no longer available.")
        .await;

    // ================= damage → wound penalty shrinks the pool ===========
    // 4 Lethal → Wounded (-2). The next strength+brawl pool is 7 - 2 = 5,
    // and the roll line carries the wound tag. +harm is staff-gated.
    let mark = nick.mark();
    nick.send_line("+harm Nick=4L").await;
    nick.expect_after(mark, "Permission denied").await;
    god.send_line("+harm Nick=4L").await;
    // Wait for the harm to LAND (God's confirmation) before Nick rolls —
    // the two are on separate connections and otherwise race in the world
    // queue, which would let the roll see the pre-damage pool.
    god.expect("Nick takes 4 lethal damage.").await;

    let mark = nick.mark();
    nick.send_line("+roll strength+brawl").await;
    nick.expect_after(mark, "(5 dice, diff 6)").await;
    nick.expect_after(mark, "wound -2").await;

    // ================= XP: award, spend, approve, applied ================
    god.send_line("+xp/award Nick=10").await;
    god.expect("Awarded 10 XP to Nick.").await;
    let mark = nick.mark();
    nick.send_line("+xp").await;
    nick.expect_after(mark, "10 XP available").await;

    // Raise brawl 3 → 4: cost = current 3 × 2 = 6 XP. Files the wod-xp job 2.
    let mark = nick.mark();
    nick.send_line("+xp/spend brawl=4").await;
    nick.expect_after(mark, "for 6 XP").await;
    god.send_line("+job/approve 2=Approved.").await;
    god.expect("Job 2 approved.").await;
    nick.expect("has been approved by God").await;

    // Applied atomically: 10 - 6 = 4 XP left, and the log records the raise.
    let mark = nick.mark();
    nick.send_line("+xp").await;
    nick.expect_after(mark, "4 XP available").await;
    nick.expect_after(mark, "brawl").await;

    // ================= +sheet renders the classic dotted sheet ===========
    let mark = nick.mark();
    nick.send_line("+sheet").await;
    nick.expect_after(mark, "WOD SHEET").await;
    nick.expect_after(mark, "Strength").await;

    // ================= help wod loaded from docs/help/wod.md ==============
    let mark = nick.mark();
    nick.send_line("help wod").await;
    nick.expect_after(mark, "World of Darkness").await;

    // Sanity on the raw stream: the ruleset output really was ANSI, and
    // stripping it yields the classic line both ways.
    assert!(
        nick.seen.contains('\u{1b}'),
        "WoD SSH output must carry ANSI escapes"
    );
    assert!(strip_ansi(&nick.seen).contains("Nick rolls strength+brawl"));

    let _ = std::fs::remove_dir_all(&dir);
}

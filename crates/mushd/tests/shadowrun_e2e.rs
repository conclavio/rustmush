//! M10b SR5 end-to-end journey on a **shadowrun-only** server
//! (`[rulesets].enabled = ["shadowrun"]`), proving the second game system rides
//! the same plugin API the WoD e2e exercises — over real SSH, in-memory store,
//! ungated, deterministic (fixed RNG seed).
//!
//! The full server is booted exactly as `main.rs` does (`Engine::with_modules`
//! → `boot_modules`) with the built-in modules **plus the SR5 ruleset** — the
//! one selection difference from the WoD e2e. Then a player builds a runner and
//! plays, all asserted on the rendered ANSI the SSH clients actually read:
//!
//! 1. a fresh player runs the **priority** `+chargen` (a combat magician),
//!    learns a spell, and `+chargen/submit`s;
//! 2. God (`+job/approve`) approves the `shadowrun-chargen` job → sheet Approved;
//! 3. a seeded `+roll` **glitches** (more than half 1s) rendering hits + the glitch;
//! 4. `+roll agility+pistols` shows the dice count and the auto **Physical limit**,
//!    and a room witness sees the public roll;
//! 5. `/edge` **pushes the limit** (adds Edge dice, waives the Limit, Rule of Six);
//! 6. `+damage` (staff) then a subsequent pool shrinks by the **wound modifier**;
//! 7. the magician `+cast`s a spell and resists **Drain**;
//! 8. `+init/roll` announces an Initiative score;
//! 9. `+karma/award` + `+karma/spend` files a costed raise; on `+job/approve` the
//!    skill goes up and the Karma is deducted;
//! 10. `+sheet` renders the SR5 sheet; `help shadowrun` loads from docs/help.
//!
//! A second, in-process test confirms the schema-generic web payload: the
//! `shadowrun.sheet` query returns a well-formed `{schema, data, approved}` and a
//! `+roll` pushes an `sr.roll` panel keyframe — so the web Sheet/roll panels
//! render SR5 with **no** web change.

mod support;

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use mush_core::{Audience, Engine, GOD, World};
use mush_proto::OutputEvent;
use mush_session::{SessionConfig, SessionManager};
use mush_store::{AccountRec, CharacterRef, InMemoryStore, ObjectStore};
use rand::{RngCore, SeedableRng};
use rand_chacha::ChaCha8Rng;

use mushd::accounts::{Accounts, hash_password};
use mushd::ipguard::IpGuard;
use mushd::ssh_bridge::{SshBridge, start_listener};
use mushd::staff::{StaffQueue, register_server_commands};
use mushd::world_task::{self, ServerDeps};
use mushd::{modules, unix_now_s};

use support::{
    ChildServer, Term, play_client, redis_url_with_db, scratch_dir, seed_boss_account, strip_ansi,
};

const GM_PASSWORD: &str = "gm-secret-6";

/// Find a seed whose ruleset RNG (`ChaCha8Rng::seed_from_u64`, each die
/// `next_u32() % 6 + 1`) makes the **first** four dice a *plain* glitch: more
/// than half are 1s and at least one is a hit (5/6). Because the glitch `+roll
/// 4` is the very first dice-rolling command in the journey, the server RNG is
/// at position 0 when it runs, so the outcome is fully determined by this seed.
fn first_roll_glitch_seed() -> u64 {
    for seed in 0u64..5_000_000 {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let faces: [u8; 4] = std::array::from_fn(|_| (rng.next_u32() % 6) as u8 + 1);
        let ones = faces.iter().filter(|&&f| f == 1).count();
        let hits = faces.iter().filter(|&&f| f >= 5).count();
        // SR5 RAW: glitch when MORE than half the dice are 1s (strictly >),
        // and at least one hit keeps it a plain (non-critical) glitch.
        if ones * 2 > 4 && hits >= 1 {
            return seed;
        }
    }
    panic!("no first-roll glitch seed found in range");
}

// ---------------------------------------------------------------------
// Boot: production wiring + built-in modules + the seeded SR5 ruleset, on an
// ephemeral port. Mirrors main.rs' boot order exactly, with the ruleset
// appended AFTER the built-ins (as ruleset_modules is, in prod) — the only
// difference from the WoD e2e is which ruleset module is selected.
// ---------------------------------------------------------------------
async fn boot_server(dir: &Path, seed: u64) -> SocketAddr {
    std::fs::create_dir_all(dir).unwrap();
    // A generous flood budget: chargen fires a ~26-line burst on one connection.
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
    // Built-ins then the SR5 ruleset (seeded), exactly as main.rs composes
    // server_modules() ++ ruleset_modules(["shadowrun"]), with a deterministic RNG.
    let mut mods = modules::server_modules();
    mods.push(Box::new(ruleset_shadowrun::ShadowrunRuleset::with_seed(
        seed,
    )));
    let mut engine = Engine::with_modules(world, registry, mods).unwrap();
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
        "Welcome to the SR5 e2e MUSH!".to_string(),
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
    term.expect("Welcome to the SR5 e2e MUSH!").await;
    term.send_line(&format!("create {name} {pw}")).await;
    term.expect(&format!("Connected as {name}")).await;
    term.expect("Limbo(#0)").await;
    (handle, term)
}

// ---------------------------------------------------------------------
// The journey
// ---------------------------------------------------------------------
#[tokio::test(flavor = "multi_thread")]
async fn sr5_full_journey_over_ssh() {
    let seed = first_roll_glitch_seed();
    let dir = scratch_dir("sr5-e2e");
    let addr = boot_server(&dir, seed).await;

    // --- God + the player + a room witness. -----------------------------
    let (_god_handle, mut god) = play_client(addr).await;
    god.expect("Welcome to the SR5 e2e MUSH!").await;
    god.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    god.expect("Connected as God").await;
    god.expect("Limbo(#0)").await;

    let (_razor_handle, mut razor) = login_create(addr, "Razor", "secret123").await;
    god.expect("Razor has connected.").await;
    // A witness in the same room, to prove the room sees a public roll.
    let (_wisp_handle, mut wisp) = login_create(addr, "Wisp", "hunter42").await;
    razor.expect("Wisp has connected.").await;

    // ================= chargen (priority point-buy: a combat magician) =====
    razor.send_line("+chargen").await;
    razor.expect("Chargen started.").await;
    // Metatype D (human, 3 special) / Attributes A (24) / Magic B (Magic 6) /
    // Skills C (28 skill + 2 group) / Resources E (6000¥).
    razor
        .send_line("+chargen/priority metatype=D attributes=A magic=B skills=C resources=E")
        .await;
    razor.expect("Priorities set.").await;

    for c in [
        "+stat metatype=human",
        "+stat awakened_type=magician",
        "+stat tradition=hermetic",
        // Attributes A = 24 (none at the human max of 6).
        "+stat body=4",
        "+stat agility=5",
        "+stat reaction=4",
        "+stat strength=3",
        "+stat charisma=2",
        "+stat intuition=4",
        "+stat logic=5",
        "+stat willpower=5",
        // Special (metatype D human = 3): Edge 2→5.
        "+stat edge=5",
        // Active skills C = 28, plus 8 free Magical-skill points (a Magic-B
        // Magician's two free Magical skills @ Rating 4). The free budget pays
        // for the first 8 points of magical-category skills, so 22 magical +
        // 14 non-magical = 36 total: 28 from the pool + 8 from the free budget.
        "+stat active_skills:spellcasting=6",
        "+stat active_skills:counterspelling=6",
        "+stat active_skills:summoning=6",
        "+stat active_skills:pistols=6",
        "+stat active_skills:perception=4",
        "+stat active_skills:sneaking=4",
        "+stat active_skills:assensing=4",
        // Skill groups C = 2.
        "+stat skill_groups:influence=2",
        // Resources E = 6000¥ (leftover cash is fine).
        "+stat nuyen=5000",
    ] {
        razor.send_line(c).await;
    }
    // The last stat's budget line confirms the whole burst landed.
    razor.expect("Set nuyen = 5000.").await;

    // Learn a spell while unapproved (chargen) so +cast has something to throw.
    razor.send_line("+spell Manabolt").await;
    razor.expect("Learned the spell").await;

    let mark = razor.mark();
    razor.send_line("+chargen/status").await;
    razor.expect_after(mark, "Budget balances").await;

    razor.send_line("+chargen/submit").await;
    razor.expect("Submitted for approval as job").await;
    let chargen_job = support::parse_id_after(&strip_ansi(&razor.seen), "as job ");

    // ================= staff approval → sheet Approved ==================
    god.send_line(&format!(
        "+job/approve {chargen_job}=Welcome to the shadows."
    ))
    .await;
    god.expect(&format!("Job {chargen_job} approved.")).await;
    razor.expect("has been approved by God").await;

    // Re-running +chargen now refuses (the character is approved).
    let mark = razor.mark();
    razor.send_line("+chargen").await;
    razor.expect_after(mark, "already approved").await;

    // ================= a seeded GLITCH (the SR5 signature) =============
    // The FIRST dice command server-wide, so the RNG is at position 0 and the
    // seed forces a plain glitch (more than half 1s, at least one hit).
    let mark = razor.mark();
    razor.send_line("+roll 4").await;
    razor.expect_after(mark, "(4 dice)").await;
    razor.expect_after(mark, "glitch (").await;
    assert!(
        !strip_ansi(&razor.seen[mark..]).contains("critical glitch"),
        "the seeded roll must be a plain glitch (has hits), not a critical glitch"
    );

    // ================= public roll: dice count + the auto Physical limit ===
    // Agility 5 + Pistols 6 = 11 dice; Physical Limit ceil((STR3*2+BOD4+REA4)/3)
    // = ceil(14/3) = 5.
    let wisp_mark = wisp.mark();
    let mark = razor.mark();
    razor.send_line("+roll agility+pistols").await;
    razor.expect_after(mark, "(11 dice)").await;
    razor.expect_after(mark, "Physical limit 5").await;
    // The witness in the room sees the same public roll (the render uses the
    // traits' display labels, "Agility+Pistols").
    wisp.expect_after(wisp_mark, "Razor rolls Agility+Pistols (11 dice)")
        .await;

    // ================= /edge pushes past the Limit ======================
    // Edge 5 → +5 dice, the Limit waived, Rule of Six.
    let mark = razor.mark();
    razor.send_line("+roll/edge agility+pistols").await;
    razor.expect_after(mark, "pushed the limit").await;
    assert!(
        !strip_ansi(&razor.seen[mark..]).contains("Physical limit"),
        "an Edge push must waive (not show) the Limit"
    );

    // ================= damage → wound penalty shrinks the pool ==========
    // 3 Physical → wound −1; the next agility+pistols pool is 11 − 1 = 10.
    // +damage is staff-gated; wait for God's confirmation before Razor rolls so
    // the two connections don't race in the world queue.
    let mark = razor.mark();
    razor.send_line("+damage Razor=3P").await;
    razor.expect_after(mark, "Permission denied").await;
    god.send_line("+damage Razor=3P").await;
    god.expect("Razor takes 3 physical damage.").await;

    let mark = razor.mark();
    razor.send_line("+roll agility+pistols").await;
    razor.expect_after(mark, "(10 dice)").await;
    razor.expect_after(mark, "wound -1").await;

    // ================= magic: +cast rolls spellcasting then Drain =======
    let mark = razor.mark();
    razor.send_line("+cast Manabolt force 3").await;
    razor.expect_after(mark, "Cast Manabolt").await;
    razor.expect_after(mark, "Drain").await;

    // ================= initiative ======================================
    let mark = razor.mark();
    razor.send_line("+init/roll").await;
    razor.expect_after(mark, "Razor rolls Initiative:").await;
    razor.expect_after(mark, "= ").await;

    // ================= Karma: award, spend, approve, applied ===========
    god.send_line("+karma/award Razor=20").await;
    god.expect("Awarded 20 Karma to Razor.").await;

    // Raise Pistols 6 → 7: cost = 7 × 2 = 14 Karma. Files the shadowrun-karma job.
    let mark = razor.mark();
    razor
        .send_line("+karma/spend active_skills:pistols=7")
        .await;
    razor.expect_after(mark, "for 14 Karma").await;
    let karma_job = support::parse_id_after(&strip_ansi(&razor.seen[mark..]), "Filed job ");
    god.send_line(&format!("+job/approve {karma_job}=Approved."))
        .await;
    god.expect(&format!("Job {karma_job} approved.")).await;
    razor.expect("has been approved by God").await;

    // Applied atomically: 20 − 14 = 6 Karma left.
    let mark = razor.mark();
    razor.send_line("+karma").await;
    razor.expect_after(mark, "6 Karma available").await;

    // ================= +sheet renders the SR5 sheet ====================
    let mark = razor.mark();
    razor.send_line("+sheet").await;
    razor.expect_after(mark, "SHADOWRUN SHEET").await;
    razor.expect_after(mark, "Body").await;

    // ================= help shadowrun loaded from docs/help ============
    let mark = razor.mark();
    razor.send_line("help shadowrun").await;
    razor.expect_after(mark, "Shadowrun 5th Edition").await;

    // Sanity on the raw stream: the ruleset output really was ANSI, and
    // stripping it yields the classic line both ways.
    assert!(
        razor.seen.contains('\u{1b}'),
        "SR5 SSH output must carry ANSI escapes"
    );
    assert!(strip_ansi(&razor.seen).contains("Razor rolls Agility+Pistols"));

    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------
// The schema-generic web payload: the `shadowrun.sheet` query returns a
// well-formed {schema, data, approved} envelope and a +roll pushes an `sr.roll`
// panel keyframe — the two feeds the web client renders with NO web change.
// Driven in-process (no web stack needed) — just confirming the payloads.
// ---------------------------------------------------------------------
#[test]
fn sr5_web_sheet_query_and_roll_panel_payload() {
    use mush_jobs::JobsModule;
    use mush_testkit::TestWorld;

    let mut tw = TestWorld::with_modules(vec![
        Box::new(JobsModule::new()),
        Box::new(ruleset_shadowrun::ShadowrunRuleset::with_seed(99)),
    ]);
    let razor = tw.spawn_player("Razor");
    // Staff give Razor a rollable body (each @sheet/set recomputes derived stats).
    for c in [
        "@sheet/set Razor/metatype=human",
        "@sheet/set Razor/agility=6",
        "@sheet/set Razor/reaction=4",
        "@sheet/set Razor/body=4",
        "@sheet/set Razor/strength=3",
        "@sheet/set Razor/active_skills:pistols=5",
    ] {
        tw.run(GOD, c);
    }

    // A +roll pushes an `sr.roll` panel keyframe to the roller, carrying the
    // rendered summary and the full per-die outcome (what the web roll panel eats).
    let events = tw.run(razor, "+roll agility+pistols");
    let panel = events
        .iter()
        .find_map(|(aud, ev)| match ev {
            OutputEvent::Panel { feed, data, .. }
                if feed == "sr.roll" && *aud == Audience::Actor(razor) =>
            {
                Some(data.clone())
            }
            _ => None,
        })
        .expect("a +roll pushes an sr.roll panel keyframe");
    assert!(
        panel.get("summary").and_then(|v| v.as_str()).is_some(),
        "sr.roll keyframe carries a rendered summary: {panel}"
    );
    assert!(
        panel.get("outcome").and_then(|o| o.get("dice")).is_some(),
        "sr.roll keyframe carries the per-die outcome: {panel}"
    );

    // The generic `<ruleset>.sheet` web query returns the schema-driven payload
    // the web Sheet panel renders unchanged: {target, schema, approved, data}.
    let (owner, handler) = tw
        .registry()
        .query_handler("shadowrun.sheet")
        .expect("the shadowrun.sheet web query is registered");
    assert_eq!(owner.as_str(), "shadowrun");
    let event = handler
        .query(tw.world(), razor, &serde_json::json!({}))
        .expect("the sheet query answers for the actor's own sheet");
    let OutputEvent::Panel { feed, data, .. } = event else {
        panic!("the sheet query must reply with a Panel keyframe");
    };
    assert_eq!(feed.as_str(), "shadowrun.sheet");
    // A well-formed SR5 schema + data payload.
    assert!(
        data.get("schema").and_then(|s| s.get("sections")).is_some(),
        "payload carries the sheet schema (sections): {data}"
    );
    assert!(
        data.get("approved").is_some(),
        "payload carries the approval state: {data}"
    );
    let sheet_data = data.get("data").expect("payload carries the sheet data");
    assert_eq!(
        sheet_data
            .get("agility")
            .and_then(serde_json::Value::as_i64),
        Some(6),
        "the staff-set Agility reached the web sheet payload: {sheet_data}"
    );
    // The two condition-monitor tracks are present (SR5-specific structure the
    // schema-generic renderer handles with no web change).
    assert!(
        sheet_data.get("condition_physical").is_some()
            && sheet_data.get("condition_stun").is_some(),
        "both SR5 Condition Monitors are in the payload: {sheet_data}"
    );
}

// ---------------------------------------------------------------------
// The gated reboot twin: a runner's sheet, persisted to Redis, survives a
// full server restart and still decodes / validates / rolls. Gated on
// REDIS_URL (the sole gated runner); skips cleanly with a note otherwise.
//
// Cross-binary note: like the durability e2e's child tests, `@shutdown`
// triggers an instance-wide BGSAVE — the gated child/reboot tests share one
// Redis instance and are meant to be run as the sole gated runner. This test
// uses its own logical db (14) and reads recovered state from Redis RAM (not
// the RDB), so it does not depend on any particular BGSAVE completing.
// ---------------------------------------------------------------------

/// A `mushd.toml` for a **shadowrun-only** server (`[rulesets].enabled =
/// ["shadowrun"]`) with the given `[store]` section body.
fn write_sr5_config(dir: &Path, store_section: &str) -> std::path::PathBuf {
    let data = dir.join("data");
    std::fs::create_dir_all(&data).unwrap();
    let path = dir.join("mushd.toml");
    // TOML basic strings treat backslash as an escape; forward slashes work
    // fine on Windows.
    let data_str = data.display().to_string().replace('\\', "/");
    let toml = format!(
        "[server]\n\
         name = \"sr5 reboot rig\"\n\
         data_dir = \"{data_str}\"\n\
         \n\
         [store]\n\
         {store_section}\n\
         \n\
         [ssh]\n\
         listen = \"127.0.0.1:0\"\n\
         \n\
         [web]\n\
         listen = \"127.0.0.1:0\"\n\
         \n\
         # The one selection that makes this a Shadowrun server.\n\
         [rulesets]\n\
         enabled = [\"shadowrun\"]\n"
    );
    std::fs::write(&path, toml).unwrap();
    path
}

#[tokio::test(flavor = "multi_thread")]
async fn sr5_sheet_survives_redis_reboot_gated() {
    let Some(base) = std::env::var("REDIS_URL").ok().filter(|s| !s.is_empty()) else {
        eprintln!(
            "skipping sr5_sheet_survives_redis_reboot_gated: REDIS_URL not set \
             (no live Redis on this machine)"
        );
        return;
    };
    let url = redis_url_with_db(&base, 14);
    let admin = mush_store::RedisStore::connect(&url)
        .await
        .expect("REDIS_URL set but connect failed");
    admin.wipe_namespaces().await.expect("wipe test db");
    admin.close().await.ok();

    let dir = scratch_dir("sr5-reboot");
    seed_boss_account(&dir.join("data"), GM_PASSWORD);
    let cfg = write_sr5_config(&dir, &format!("store = \"redis\"\nredis_url = \"{url}\""));

    // --- first life: staff build a runner, then a graceful shutdown that
    //     drains every pending write durably to Redis before exit 0. ---------
    let mut server = ChildServer::spawn(&cfg);
    let addr = server.ssh_addr().await;

    let (_god_handle, mut god) = play_client(addr).await;
    god.expect("connect <account> <password>").await;
    god.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    god.expect("Connected as God").await;
    god.expect("Limbo(#0)").await;

    let (_razor_handle, mut razor) = play_client(addr).await;
    razor.send_line("create Razor secret123").await;
    razor.expect("Connected as Razor").await;
    razor.expect("Limbo(#0)").await;
    god.expect("Razor has connected.").await;

    // Each @sheet/set recomputes the SR5 derived stats (Limits, Condition
    // Monitors, Initiative), so the persisted sheet is fully consistent.
    for c in [
        "@sheet/set Razor/metatype=human",
        "@sheet/set Razor/body=5",
        "@sheet/set Razor/agility=6",
        "@sheet/set Razor/reaction=4",
        "@sheet/set Razor/strength=3",
        "@sheet/set Razor/willpower=4",
        "@sheet/set Razor/active_skills:pistols=5",
    ] {
        god.send_line(c).await;
    }
    god.expect("Set Razor/active_skills:pistols = 5.").await;

    god.send_line("@shutdown").await;
    god.expect("GAME: Shutdown in progress.").await;
    let status = server.wait_exit(Duration::from_secs(30)).await;
    assert!(
        status.success(),
        "clean shutdown must exit 0, got {status:?}; log:\n{}",
        server.log_snapshot()
    );
    drop(god);
    drop(razor);

    // --- second life: a fresh process loads the world from the same Redis. --
    let mut server2 = ChildServer::spawn(&cfg);
    let addr2 = server2.ssh_addr().await;

    let (_razor2_handle, mut razor2) = play_client(addr2).await;
    // The account survived — the runner logs back in.
    razor2.send_line("connect Razor secret123").await;
    razor2.expect("Connected as Razor").await;
    razor2.expect("Limbo(#0)").await;

    // The sheet survived, decodes, and validates: it renders...
    let mark = razor2.mark();
    razor2.send_line("+sheet").await;
    razor2.expect_after(mark, "SHADOWRUN SHEET").await;
    // ...and it rolls — Agility 6 + Pistols 5 = 11 dice, with the derived
    // Physical Limit ceil((STR3*2 + BOD5 + REA4)/3) = 5 intact.
    let mark = razor2.mark();
    razor2.send_line("+roll agility+pistols").await;
    razor2.expect_after(mark, "(11 dice)").await;
    razor2.expect_after(mark, "Physical limit 5").await;

    server2.kill_hard();
    let _ = std::fs::remove_dir_all(&dir);
}

//! M10a oWoD20 **secondary splats** end-to-end over real SSH (in-memory store,
//! ungated): the same production wiring as `wod_splats_e2e.rs` (world task,
//! session layer, accounts, SSH listener, built-ins + the seeded WoD ruleset)
//! driving three M10a characters — a **Changeling** (C20), a **Ghoul** (V20
//! mortal-plus) and a **Kinfolk** (W20 mortal-plus) — through the per-template
//! dispatch:
//!
//! 1. each character runs `+chargen`, picks their **template** with
//!    `+chargen/template`, allocates their splat traits, and is staff-approved;
//! 2. `+sheet` renders **only** that splat's sections (a changeling has the Fae
//!    block + Arts/Realms but no Disciplines/Spheres/Garou; a ghoul has
//!    Disciplines + a Blood Pool but no Spheres/Garou/Fae; a kinfolk has the
//!    Kinfolk block + Gifts but no Disciplines/Rage/Spheres) — the dispatch
//!    acceptance test;
//! 3. the Changeling `+cantrip`s (rolling the Art pool alone, the Realm a
//!    level-gate), the Ghoul `+disc/roll`s
//!    Potence and `+blood/spend`s (reusing the vampire `+disc`/`+blood`
//!    surface, gate-widened to serve ghouls), and the Kinfolk `+gnosis` +
//!    `+gift/roll`s (reusing the werewolf `+gnosis`/`+gift` surface,
//!    gate-widened to serve Kinfolk);
//! 4. **cross-splat refusal**: a Changeling typing `+blood` is told they are
//!    not Kindred, a Ghoul typing `+cantrip` is not Kithain, a Kinfolk typing
//!    `+disc` is not Kindred, and a plain Mortal has access to none of them.
//!
//! The ruleset is seeded so the run is deterministic; dice assertions are
//! structural (the pool's dice count is fully determined by traits, independent
//! of how many draws earlier commands consumed).

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
const S: u64 = 24680;
const GM_PASSWORD: &str = "gm-secret-more-splats";

// ---------------------------------------------------------------------
// Boot: production wiring + built-in modules + the seeded WoD ruleset,
// on an ephemeral port (mirrors wod_splats_e2e.rs boot order).
// ---------------------------------------------------------------------

async fn boot_server(dir: &Path) -> SocketAddr {
    std::fs::create_dir_all(dir).unwrap();
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
            ssh_keys: vec![],
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
            metrics: std::sync::Arc::new(mush_net_web::Metrics::new()),
            store: Arc::clone(&store),
            guard: Arc::new(IpGuard::new()),
            staff,
            llm: None,
            quarantine_dir: None,
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
        "Welcome to the more-splats e2e MUSH!".to_string(),
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
    term.expect("Welcome to the more-splats e2e MUSH!").await;
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
// Chargen scripts (the balanced base allocation proven in wod_splats_e2e.rs,
// driven here over the SSH front door).
// ---------------------------------------------------------------------

/// The shared, budget-balancing base allocation (attributes / abilities) used
/// by all three splats after the priorities are set.
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
/// the given splat-specific setup lines and the final +stat marker to fence on.
/// `job` is the wod-chargen job id this submit will open.
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
async fn m10a_wod_more_splats_full_journey() {
    let dir = scratch_dir("wod-more-splats-e2e");
    let addr = boot_server(&dir).await;

    // --- God + the three splat characters + a plain mortal. --------------
    let (_god_handle, mut god) = play_client(addr).await;
    god.expect("Welcome to the more-splats e2e MUSH!").await;
    god.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    god.expect("Connected as God").await;
    god.expect("Limbo(#0)").await;

    let (_nyx_handle, mut nyx) = login_create(addr, "Nyx", "secret123").await;
    god.expect("Nyx has connected.").await;
    let (_ren_handle, mut ren) = login_create(addr, "Renfield", "secret123").await;
    god.expect("Renfield has connected.").await;
    let (_ash_handle, mut ash) = login_create(addr, "Ash", "secret123").await;
    god.expect("Ash has connected.").await;
    let (_norm_handle, mut norm) = login_create(addr, "Norm", "secret123").await;
    god.expect("Norm has connected.").await;
    let (_kit_handle, mut kit) = login_create(addr, "Kit", "secret123").await;
    god.expect("Kit has connected.").await;

    // ================= Changeling (Pooka / Childling) chargen → job 1 =========
    chargen_and_approve(
        &mut nyx,
        &mut god,
        "changeling",
        &[
            "+stat kith=pooka",
            "+stat seeming=childling",
            "+stat court=seelie",
        ],
        &[
            "+stat backgrounds:Chimera=3",
            "+stat backgrounds:Contacts=2",
            // Arts: 3-dot pool.
            "+stat arts:Chicanery=2",
            "+stat arts:Wayfare=1",
            // Realms: 5-dot pool.
            "+stat realms:Actor=3",
            "+stat realms:Fae=2",
        ],
        "Set realms:Fae = 2.",
        1,
    )
    .await;

    // ================= Ghoul (Domitor: Brujah) chargen → job 2 ================
    chargen_and_approve(
        &mut ren,
        &mut god,
        "ghoul",
        &["+stat domitor_clan=brujah", "+stat domitor=Prince Marcus"],
        &[
            "+stat backgrounds:Resources=3",
            "+stat backgrounds:Contacts=2",
            // The 1-dot Discipline pool (Potence is always in-set).
            "+stat disciplines:Potence=1",
            // Virtues cost freebies for a ghoul (no Virtue pool): 3 each = 12.
            "+stat conscience=3",
            "+stat self_control=3",
            "+stat courage=3",
        ],
        "Set courage = 3.",
        2,
    )
    .await;

    // ================= Kinfolk (Fianna) chargen → job 3 ======================
    chargen_and_approve(
        &mut ash,
        &mut god,
        "kinfolk",
        &["+stat tribe=fianna"],
        &[
            // Gnosis + Pure Breed ride the Backgrounds map (no dedicated pool).
            "+stat backgrounds:Gnosis=2",
            "+stat backgrounds:Pure Breed=1",
            "+stat backgrounds:Contacts=2",
        ],
        "Set backgrounds:Contacts = 2.",
        3,
    )
    .await;

    // ================= +sheet dispatch: each carries ONLY its splat ==========
    let csheet = sheet_text(&mut nyx).await;
    assert!(csheet.contains("FAE"), "changeling Fae block: {csheet}");
    assert!(csheet.contains("ARTS"), "changeling Arts section");
    assert!(csheet.contains("REALMS"), "changeling Realms section");
    assert!(csheet.contains("Glamour"), "changeling Glamour");
    assert!(
        !csheet.contains("DISCIPLINES"),
        "a changeling is not Kindred"
    );
    assert!(!csheet.contains("SPHERES"), "a changeling is not a mage");
    assert!(!csheet.contains("GAROU"), "a changeling is not Garou");

    let gsheet = sheet_text(&mut ren).await;
    assert!(gsheet.contains("GHOUL"), "ghoul block: {gsheet}");
    assert!(gsheet.contains("DISCIPLINES"), "ghoul Disciplines");
    assert!(gsheet.contains("Blood Pool"), "ghoul Blood Pool");
    assert!(!gsheet.contains("SPHERES"), "a ghoul is not a mage");
    assert!(!gsheet.contains("GAROU"), "a ghoul is not Garou");
    assert!(!gsheet.contains("FAE"), "a ghoul is not Kithain");

    let ksheet = sheet_text(&mut ash).await;
    assert!(ksheet.contains("KINFOLK"), "kinfolk block: {ksheet}");
    assert!(ksheet.contains("GIFTS"), "kinfolk Gifts section");
    assert!(ksheet.contains("Gnosis"), "kinfolk Gnosis");
    assert!(!ksheet.contains("DISCIPLINES"), "kinfolk is not Kindred");
    assert!(!ksheet.contains("SPHERES"), "kinfolk is not a mage");
    assert!(!ksheet.contains("Rage"), "kinfolk has no Rage");

    // ================= Changeling: +cantrip (rolls the Art alone) ============
    // C20: the cantrip rolls the Art pool alone — Chicanery 2 → 2 dice at
    // difficulty 6; the Realm (Actor) gates but contributes no dice.
    let mark = nyx.mark();
    nyx.send_line("+cantrip chicanery+actor vs 6").await;
    nyx.expect_after(mark, "cantrip (chicanery, 2 dice, diff 6)")
        .await;
    // A Realm the changeling has no dots in is refused (still a level-gate).
    let mark = nyx.mark();
    nyx.send_line("+cantrip chicanery+time").await;
    nyx.expect_after(mark, "no dots in the Realm of Time").await;
    // Glamour: Childling starts with 5; spend 2 → 3/5.
    let mark = nyx.mark();
    nyx.send_line("+glamour/spend 2").await;
    nyx.expect_after(mark, "Glamour 3/5").await;

    // ================= Ghoul: +disc/roll Potence + +blood/spend =============
    // Potence 1 (free grant) + Strength 4 = 5 dice at difficulty 6.
    let mark = ren.mark();
    ren.send_line("+disc/roll potence+strength vs 6").await;
    ren.expect_after(mark, "discipline (potence+strength, 5 dice, diff 6)")
        .await;
    // A freshly-ghouled Blood Pool starts partial at 3; spend 2 → 1/10.
    let mark = ren.mark();
    ren.send_line("+blood/spend 2").await;
    ren.expect_after(mark, "Blood 1/10").await;
    // +domitor shows the domitor + their clan.
    let mark = ren.mark();
    ren.send_line("+domitor").await;
    ren.expect_after(mark, "Prince Marcus").await;
    // A ghoul spends vitae to heal (V20). Top the pool back up, take lethal, and
    // mend it with +blood/heal (the vampire +blood registration's heal switch,
    // delegated to the ghoul). Blood is 1/10 after the spend above.
    let gmark = god.mark();
    god.send_line("+harm Renfield=3L").await;
    god.expect_after(gmark, "takes 3 lethal").await;
    let mark = ren.mark();
    ren.send_line("+blood/gain 3").await;
    ren.expect_after(mark, "Blood 4/10").await;
    let mark = ren.mark();
    ren.send_line("+blood/heal 2").await;
    ren.expect_after(mark, "mending 2 health level(s). Blood 2/10")
        .await;

    // ================= Kinfolk: +gnosis + +gift/roll ========================
    // Gnosis derived from the 2-dot Gnosis background.
    let mark = ash.mark();
    ash.send_line("+gnosis").await;
    ash.expect_after(mark, "Gnosis: 2/2").await;
    // Staff grant a level-1 Gift, then the Kinfolk rolls its pool.
    let gmark = god.mark();
    god.send_line("@sheet/set Ash/gifts:Persuasion=1").await;
    god.expect_after(gmark, "Set Ash/gifts:Persuasion").await;
    // Charisma 3 + Empathy 2 = 5 dice at difficulty 6.
    let mark = ash.mark();
    ash.send_line("+gift/roll persuasion=charisma+empathy vs 6")
        .await;
    ash.expect_after(mark, "persuasion (charisma+empathy, 5 dice, diff 6)")
        .await;

    // ================= Cross-splat refusal (self-gating commands) ============
    // A Changeling typing +blood is told they are not Kindred.
    let mark = nyx.mark();
    nyx.send_line("+blood").await;
    nyx.expect_after(mark, "not Kindred").await;
    // A Ghoul typing +cantrip is not Kithain.
    let mark = ren.mark();
    ren.send_line("+cantrip chicanery+actor").await;
    ren.expect_after(mark, "not Kithain").await;
    // A Kinfolk typing +disc is not Kindred.
    let mark = ash.mark();
    ash.send_line("+disc").await;
    ash.expect_after(mark, "not Kindred").await;
    // The +gnosis/+gift gate widened to Garou-OR-Kinfolk, but the strictly-Garou
    // commands stayed narrow: a Kinfolk is refused +rage / +shift / +renown /
    // +rank ("not Garou"), proving the widening touched only Gnosis/Gifts.
    for strict in ["+rage", "+shift crinos", "+renown", "+rank"] {
        let mark = ash.mark();
        ash.send_line(strict).await;
        ash.expect_after(mark, "not Garou").await;
    }

    // ================= Over-claim fences (roll a power you lack) =============
    // A Ghoul's +disc/roll naming a Discipline it lacks is refused at the
    // command's gate (Renfield knows only Potence).
    let mark = ren.mark();
    ren.send_line("+disc/roll dominate+wits vs 6").await;
    ren.expect_after(mark, "Name a Discipline you have").await;
    // A Kinfolk's +gift/roll naming an unknown Gift is refused (Ash knows only
    // the staff-granted Persuasion).
    let mark = ash.mark();
    ash.send_line("+gift/roll razor claws=dexterity+brawl vs 6")
        .await;
    ash.expect_after(mark, "do not know the Gift").await;

    // A plain Mortal has access to none of these splat surfaces.
    let mark = norm.mark();
    norm.send_line("+blood").await;
    norm.expect_after(mark, "not Kindred").await;
    let mark = norm.mark();
    norm.send_line("+cantrip chicanery+actor").await;
    norm.expect_after(mark, "not Kithain").await;
    let mark = norm.mark();
    norm.send_line("+gift").await;
    norm.expect_after(mark, "not Garou or Kinfolk").await;

    // ================= Kinfolk chargen: Gifts are locked ====================
    // Kinfolk grant no free Gifts and add no Gift pool, so a chargen
    // `+stat gifts:X=1` must be refused — otherwise a player could mint
    // arbitrarily many level-1 Gifts that no budget ledger charges.
    kit.send_line("+chargen").await;
    kit.expect("Chargen started.").await;
    kit.send_line("+chargen/template kinfolk").await;
    kit.expect("Template set to kinfolk").await;
    let mark = kit.mark();
    kit.send_line("+stat gifts:Persuasion=1").await;
    kit.expect_after(mark, "not a chargen-allocatable trait")
        .await;

    // ================= raw stream really was ANSI ===========================
    assert!(
        nyx.seen.contains('\u{1b}'),
        "WoD SSH output must carry ANSI escapes"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Drive the guided M9 wizard (`+chargen/wizard`) from the Template step through
/// `submit`, building a balanced Changeling. Returns the wizard-reported
/// `wod-chargen` job id. This exercises the wizard's per-template seams for a
/// **new** M10a template end to end: the Template step lists `changeling` with
/// its one-line blurb, the Splat step drives the Fae fields plus the **Arts**
/// power pool (`wizard_pool_field` = `"arts"`, routed to `wizard_set_pool` →
/// `apply_art`), and the Realms pool rides the generic `realms:<name>=<n>` path
/// in the Freebies step (Changeling exposes only one wizard pool slot).
async fn wizard_changeling_and_submit(t: &mut Term) -> u64 {
    t.send_line("+chargen/wizard").await;
    t.expect("Guided Character Creation").await;
    // The Template step lists the new template with its blurb.
    t.expect("changeling").await;

    t.send_line("changeling").await;
    t.expect("Template set to changeling").await;

    // Concept + priorities + attributes + abilities + backgrounds (balanced).
    for line in [
        "next",
        "concept=Grinning Trickster",
        "next",
        "physical=7 social=5 mental=3",
        "next",
        "strength=4",
        "dexterity=4",
        "stamina=2",
        "charisma=3",
        "manipulation=3",
        "appearance=2",
        "perception=2",
        "intelligence=2",
        "wits=2",
        "next",
        "talents=13 skills=9 knowledges=5",
        "next",
        "alertness=3",
        "athletics=3",
        "brawl=3",
        "empathy=2",
        "expression=2",
        "drive=3",
        "firearms=3",
        "stealth=3",
        "academics=3",
        "computer=2",
        "next",
        "Chimera=3",
        "Contacts=2",
    ] {
        t.send_line(line).await;
    }

    // Splat step: the Fae identity fields + the Arts power pool (bare names route
    // to the changeling's wizard_set_pool).
    t.send_line("next").await;
    t.send_line("kith=pooka").await;
    t.send_line("seeming=childling").await;
    t.send_line("court=seelie").await;
    t.send_line("Chicanery=2").await;
    t.expect("Arts").await;
    t.send_line("Wayfare=1").await;

    // Virtues (leave at default — Changeling's flat Willpower is exempt), then
    // the Realms pool via the generic field:key path in the Freebies step.
    t.send_line("next").await; // → Virtues
    t.send_line("next").await; // → Freebies
    t.send_line("realms:Actor=3").await;
    t.send_line("realms:Fae=2").await;
    t.send_line("next").await; // → Review
    t.send_line("submit").await;
    t.expect("Submitted for approval as job").await;

    let hay = strip_ansi(&t.seen);
    let idx = hay
        .find("Submitted for approval as job ")
        .expect("submit confirmation present");
    let tail = &hay[idx + "Submitted for approval as job ".len()..];
    let digits: String = tail.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().expect("job id parses")
}

#[tokio::test(flavor = "multi_thread")]
async fn m10a_wizard_drives_a_changeling() {
    let dir = scratch_dir("wod-more-splats-wizard");
    let addr = boot_server(&dir).await;

    let (_god_handle, mut god) = play_client(addr).await;
    god.expect("Welcome to the more-splats e2e MUSH!").await;
    god.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    god.expect("Connected as God").await;
    god.expect("Limbo(#0)").await;

    let (_puck_handle, mut puck) = login_create(addr, "Puck", "secret123").await;
    god.expect("Puck has connected.").await;

    // Build a Changeling entirely through the guided wizard, then staff-approve.
    let job = wizard_changeling_and_submit(&mut puck).await;
    let gmark = god.mark();
    god.send_line(&format!("+job/approve {job}=Welcome to the Dreaming."))
        .await;
    god.expect_after(gmark, &format!("Job {job} approved."))
        .await;
    puck.expect("has been approved by God").await;

    // The wizard-built sheet is a real Changeling: the finalize derived Glamour
    // from the Seeming (Childling → 5) and rendered the Fae block; the
    // wizard-allocated Arts survived.
    let sheet = sheet_text(&mut puck).await;
    assert!(
        sheet.contains("FAE"),
        "wizard changeling Fae block: {sheet}"
    );
    assert!(sheet.contains("ARTS"), "wizard changeling Arts");
    assert!(sheet.contains("REALMS"), "wizard changeling Realms");
    // Chicanery 2 (Art) → a working 2-die cantrip pool; Actor (Realm) gates only.
    let mark = puck.mark();
    puck.send_line("+cantrip chicanery+actor vs 6").await;
    puck.expect_after(mark, "cantrip (chicanery, 2 dice, diff 6)")
        .await;
    // Childling Glamour derived to 5.
    let mark = puck.mark();
    puck.send_line("+glamour").await;
    puck.expect_after(mark, "Glamour: 5/5").await;

    let _ = std::fs::remove_dir_all(&dir);
}

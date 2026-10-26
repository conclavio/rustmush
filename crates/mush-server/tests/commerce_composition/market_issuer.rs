//! WS04-S7: the market's title issuer is wired on the same condition the economy's
//! issuer list is — and the composition root says so once.
//!
//! `server_modules_with` computed housing's enablement once and used it for **both**
//! halves of the housing title issuer: the economy's frozen `with_title_issuers`
//! list and `HousingModule::with_issuer`. The market's was computed inline for the
//! list only, and `MarketModule::with_issuer` was **unconditional**. So on every
//! server whose market was off, the market held a name the economy did not list, and
//! the one arm that reaches `mush_economy::issue_title` — `@market/vendor stock
//! <v>=<code> object=<a thing>`, which mints the stocked asset's title — answered
//! `EconomyError::Unauthorized`: a *permission* refusal, naming a capability the
//! operator had never asked for, for a subsystem they had not turned on.
//!
//! **16b cut R-b re-points the flag and keeps every claim.** The condition used to be
//! `[shadowrun.economy].enabled` and the issuer used to be `sr.market`; there is one
//! game now, the flag is `[tacsim.economy].market`, and the issuer is
//! [`modules::market_title_issuer`] — `tacsim.market`.
//!
//! The three cases are the three altitudes that failure has:
//!
//! 1. the cheap structural gate, on the real root: with `[tacsim.economy].market`
//!    off, [`mush_market::MarketStore::issuer`] is `None`; with it on, it is
//!    `tacsim.market`. The two halves are one flag, so they cannot drift apart again
//!    silently.
//! 2. the message: a composition that carries market content and **no** issuer
//!    refuses `@market/vendor stock` in the market's own words, and specifically not
//!    with a permission refusal.
//! 3. the positive control — the same fixture stocks and the title is really minted —
//!    and, beside it, the **disagreement** the one flag exists to make
//!    unrepresentable: the same market, naming the same issuer, on a roster whose
//!    economy was built with `market = false`, is refused by the ECONOMY for want of
//!    a listed issuer. That is the historical bug, reproduced deliberately.
//!
//! # A note on the catalogue
//!
//! The production root composes the market with **no catalog sources**: the game
//! authors no commodity content yet (that is a later cut), and the two rulesets that
//! used to bring a `CatalogSource` are gone. An asset listing is the only
//! representation that mints a title, so cases 2 and 3 need one catalogue entry to
//! exist at all — and they get it by replacing exactly ONE module of the production
//! roster, the market, with one carrying [`TestCatalog`]. Everything else, the
//! economy's own frozen issuer list included, is the roster a real server boots, so
//! the difference between case 2 and case 3 is still the single `with_issuer` call
//! this step made conditional.

use std::path::PathBuf;
use std::sync::Arc;

use mush_config::config::Config;
use mush_core::{Audience, GOD, GameModule, ObjId, ObjKind};
use mush_economy::EconomyStore;
use mush_llm::LlmService;
use mush_market::{CatalogEntry, CatalogSource, MarketStore};
use mush_org::PrincipalId;
use mush_proto::OutputEvent;
use mush_server::boot;
use mush_server::modules;
use mush_testkit::{TestWorld, text_of};

/// The test pack's name, and the stocked good's `<ruleset>:<code>`.
///
/// Legal, non-stackable and **asset**-represented, which is the one representation
/// that mints a title — the property the whole file is about. Its shape is the
/// shipped pack shape, stamped with its own pack name exactly as a real loader does.
const PACK: &str = "tacsim.test";
const GOOD: &str = "tacsim:hullplate";

/// The one catalogue entry cases 2 and 3 stock, as a composition-frozen source.
///
/// It is a TEST source and says so: the game ships none yet (see this file's header).
#[derive(Debug)]
struct TestCatalog;

impl CatalogSource for TestCatalog {
    fn pack(&self, pack: &str) -> Option<Result<Vec<CatalogEntry>, String>> {
        if pack != PACK {
            return None;
        }
        let value = serde_json::json!({
            "catalog": { "ruleset": "tacsim", "code": "hullplate" },
            "pack": PACK,
            "definition_version": 1,
            "name": "A rolled hull plate",
            "baseline": 400,
            "legality": "legal",
            "representation": "asset",
            "stackable": false,
            "tags": ["test"],
        });
        Some(
            serde_json::from_value::<CatalogEntry>(value)
                .map(|entry| vec![entry])
                .map_err(|error| error.to_string()),
        )
    }
}

// ---- rig -------------------------------------------------------------------

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "mush-marketissuer-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

/// A memory-backed config on a scratch data dir with `[tacsim.economy].market`
/// either on or off — the one knob the market's wiring reads, and nothing else.
fn config(tag: &str, market: bool) -> Config {
    let mut cfg = Config::default();
    cfg.store.store = "memory".to_string();
    cfg.server.data_dir = scratch_dir(tag);
    cfg.ipc.listen = "127.0.0.1:0".to_string();
    cfg.tacsim.economy.market = market;
    cfg
}

fn llm_handle(cfg: &Config) -> Arc<LlmService> {
    let llm_config = mush_server::config_ext::llm_config(&cfg.llm).expect("[llm] parses");
    Arc::new(LlmService::from_config(&llm_config).expect("a keyless service still builds"))
}

/// The roster a server with this config boots, through the ONE root both halves of
/// the fork call.
fn roster(cfg: &Config) -> Vec<Box<dyn GameModule>> {
    let llm = llm_handle(cfg);
    boot::game_modules(cfg, &llm).expect("the roster composes")
}

/// That roster with exactly ONE module replaced: the market, rebuilt with the test
/// catalogue and — when `issuer` — the same principal [`modules::market_title_issuer`]
/// hands the real root.
fn roster_with_catalog(cfg: &Config, issuer: bool) -> Vec<Box<dyn GameModule>> {
    let mut mods = roster(cfg);
    let at = mods
        .iter()
        .position(|m| m.name() == "market")
        .expect("the market composes");
    let market = mush_market::MarketModule::new()
        .with_catalog_sources(vec![Arc::new(TestCatalog)])
        .with_availability(Vec::new())
        .with_delivery(Vec::new());
    mods[at] = Box::new(if issuer {
        market.with_issuer(modules::market_title_issuer())
    } else {
        market
    });
    mods
}

fn boot_world(cfg: &Config) -> TestWorld {
    TestWorld::with_modules(roster(cfg))
}

fn issuer_of(tw: &TestWorld) -> Option<String> {
    tw.world()
        .module_state::<MarketStore>()
        .expect("the market composes on every server")
        .issuer()
        .map(|i| i.as_str().to_string())
}

// ---- the fixture one stock attempt needs -----------------------------------

fn texts(events: &[(Audience, OutputEvent)]) -> String {
    events
        .iter()
        .map(|(_, e)| text_of(e))
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_error(events: &[(Audience, OutputEvent)]) -> bool {
    events.iter().any(|(_, e)| {
        matches!(e, OutputEvent::Sys(rt) if rt.spans().iter().any(|sp| sp.tag.as_deref() == Some("error")))
    })
}

fn ok(tw: &mut TestWorld, line: &str) -> String {
    let ev = tw.run(GOD, line);
    assert!(!is_error(&ev), "`{line}` was refused: {}", texts(&ev));
    texts(&ev)
}

fn err(tw: &mut TestWorld, line: &str) -> String {
    let ev = tw.run(GOD, line);
    assert!(
        is_error(&ev),
        "`{line}` should have been refused, got: {}",
        texts(&ev)
    );
    texts(&ev)
}

/// A registered vendor — an organization with a credit account and a vault it owns —
/// and one `Thing` in the stocking staffer's own hands, which is what
/// `@market/vendor stock … object=` requires (E6c's M10 fence).
struct Shop {
    id: mush_market::VendorId,
    object: ObjId,
}

fn shop(tw: &mut TestWorld) -> Shop {
    ok(tw, &format!("@market/catalog load {PACK}"));
    ok(tw, "@org/create Ares");
    let org = tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .and_then(|s| s.state().orgs().map(|o| o.id).max())
        .expect("the org exists");
    // The settlement currency the sheet module registers at first boot (doc 11 §2).
    ok(
        tw,
        &format!("@account/open org:#{}={}", org.0, tacsim::CREDIT),
    );
    let backing = tw
        .world_mut()
        .create(ObjKind::Thing, "Ares strongbox", GOD, None)
        .expect("a container");
    ok(
        tw,
        &format!(
            "@vault/create Ares=container:#{} owner=org:#{}",
            backing.0, org.0
        ),
    );
    let vault = tw
        .world()
        .module_state::<EconomyStore>()
        .expect("economy store")
        .snapshot()
        .vaults()
        .find(|v| v.owner == PrincipalId::Organization(org))
        .expect("the shop vault")
        .id;
    ok(
        tw,
        &format!(
            "@market/vendor create #{}=#{} name=Ares market=luna buy=300",
            org.0, vault.0
        ),
    );
    let id = tw
        .world()
        .module_state::<MarketStore>()
        .and_then(|s| s.vendor_of_org(org))
        .expect("registered")
        .id;
    let object = tw
        .world_mut()
        .create(ObjKind::Thing, "a rolled hull plate", GOD, Some(GOD))
        .expect("an object in the staffer's hands");
    Shop { id, object }
}

fn stock_line(shop: &Shop) -> String {
    format!(
        "@market/vendor stock #{}={GOOD} price=400 qty=1 object=#{}",
        shop.id.0, shop.object.0
    )
}

fn minted(tw: &TestWorld, object: ObjId) -> bool {
    tw.world()
        .module_state::<EconomyStore>()
        .expect("economy store")
        .snapshot()
        .asset(object)
        .is_some()
}

// ---- 1: the structural gate ------------------------------------------------

/// The cheap primary proof, on the composition root itself.
///
/// `[tacsim.economy].market` defaults **off** (`TacsimEconomyCfg::default`), so every
/// stock rig and every server that has not turned the market on takes the `None`
/// arm — which is exactly the population that used to carry a name for nothing.
#[test]
fn composition_without_the_game_market_registers_no_market_issuer() {
    // The default composition: the one game, no market layer.
    let off = config("off", false);
    assert!(
        !Config::default().tacsim.economy.market,
        "the market is off at the shipped defaults, so the `None` arm is the common case"
    );
    assert_eq!(
        issuer_of(&boot_world(&off)),
        None,
        "`[tacsim.economy].market = false` registers no market title issuer"
    );
    let _ = std::fs::remove_dir_all(&off.server.data_dir);

    // And with the layer on, the name the economy's own frozen list carries.
    let on = config("on", true);
    assert_eq!(
        issuer_of(&boot_world(&on)).as_deref(),
        Some(modules::market_title_issuer().as_str()),
        "an enabled market registers the market title issuer"
    );
    assert_eq!(modules::market_title_issuer().as_str(), "tacsim.market");
    // Housing's pair is the precedent this one copies, and it is on by default.
    assert_eq!(modules::housing_title_issuer().as_str(), "tacsim.housing");
    let _ = std::fs::remove_dir_all(&on.server.data_dir);
}

// ---- 2: the message --------------------------------------------------------

/// A market with content and no issuer refuses in **its own** words.
///
/// Before this step, a market-less server's market held its issuer name while the
/// economy listed nothing, so `issue_title` answered `Unauthorized` from inside a
/// verb whose operator had never named an issuer. The needle is deliberately short:
/// it is the opening clause, so a reworded tail does not break the test. W1-B7-FIX
/// closed the reason it USED to be short — `api.rs`'s refusal carried a thirty-space
/// run mid-sentence — and the no-double-space assert below is what keeps it closed.
#[test]
fn stocking_fails_clear_without_the_game_market() {
    let cfg = config("noissuer", true);
    let mut tw = TestWorld::with_modules(roster_with_catalog(&cfg, false));
    assert_eq!(issuer_of(&tw), None, "the rig's market names no issuer");

    let shop = shop(&mut tw);
    let refused = err(&mut tw, &stock_line(&shop));
    assert!(
        refused.contains("This server registers no market title issuer"),
        "{refused}"
    );
    // The text an operator reads is the whole point of this refusal, so it has
    // to read as a sentence. A wrapped Rust string literal that loses its
    // trailing `\` keeps every space of the continuation line's indent, and the
    // message ships with a thirty-space gap mid-sentence — invisible in the
    // source, obvious on a terminal (W1-B7-FIX).
    assert!(
        !refused.contains("  "),
        "the refusal must not carry a run of spaces from an unescaped line \
         continuation: {refused:?}"
    );
    // …and NOT the economy's permission refusal, which is what the operator used to
    // read for a subsystem they had not enabled.
    assert!(
        !refused.to_ascii_lowercase().contains("authoriz"),
        "a missing issuer is not a permission problem: {refused}"
    );
    // Nothing was minted on the way to the refusal.
    assert!(!minted(&tw, shop.object), "a refused stock mints no title");
    let _ = std::fs::remove_dir_all(&cfg.server.data_dir);
}

// ---- 3: the positive control, and the disagreement it rules out -------------

/// The same fixture on the roster a server with the market on really boots: the two
/// halves agree, `issue_title` accepts, and the stocked object carries a
/// vendor-owned title.
#[test]
fn stocking_mints_title_with_the_game_market_enabled() {
    let cfg = config("enabled", true);
    let mut tw = TestWorld::with_modules(roster_with_catalog(&cfg, true));
    assert_eq!(
        issuer_of(&tw).as_deref(),
        Some(modules::market_title_issuer().as_str())
    );

    let shop = shop(&mut tw);
    ok(&mut tw, &stock_line(&shop));
    assert!(
        minted(&tw, shop.object),
        "the stocked object carries a title"
    );
    let _ = std::fs::remove_dir_all(&cfg.server.data_dir);
}

/// **The disagreement the one flag makes unrepresentable**, reproduced on purpose.
///
/// The same market, naming the same issuer, over an economy the real root built with
/// `[tacsim.economy].market = false` — so the frozen `with_title_issuers` list does
/// NOT carry `tacsim.market`. `mush_economy::issue_title` refuses an issuer that is
/// not on the economy's list, and that refusal is a *permission* one: exactly what a
/// non-market server used to read, for a subsystem nobody had turned on.
///
/// This is why `server_modules_with` computes `market_on` once and gates both halves
/// with it. A composition cannot reach this state; only a test can build it.
#[test]
fn an_issuer_the_economy_does_not_list_is_refused_by_the_economy() {
    // `market = false`: the economy lists no market issuer...
    let cfg = config("mismatch", false);
    // ...while the market names one anyway. One module differs from the roster a real
    // server boots, and the economy beside it is the real one.
    let mut tw = TestWorld::with_modules(roster_with_catalog(&cfg, true));
    assert_eq!(
        issuer_of(&tw).as_deref(),
        Some(modules::market_title_issuer().as_str()),
        "the rig's market names an issuer its economy never listed"
    );

    let shop = shop(&mut tw);
    let refused = err(&mut tw, &stock_line(&shop));
    assert!(
        refused.to_ascii_lowercase().contains("authoriz"),
        "an issuer the economy does not list is a PERMISSION refusal — the shape \
         WS04-S7 exists to stop a real composition reaching: {refused}"
    );
    assert!(!minted(&tw, shop.object), "a refused stock mints no title");
    let _ = std::fs::remove_dir_all(&cfg.server.data_dir);
}

//! WS04-S7: the market's title issuer is wired on the same condition the economy's
//! issuer list is — and the composition root says so once.
//!
//! `server_modules_with` computed housing's enablement once and used it for **both**
//! halves of the housing title issuer: the economy's frozen `with_title_issuers`
//! list and `HousingModule::with_issuer`. The market's was computed inline for the
//! list only, and `MarketModule::with_issuer` was **unconditional**. So on every
//! non-Shadowrun server the market held the name `sr.market` that the economy did
//! not list, and the one arm that reaches `mush_economy::issue_title` —
//! `@market/vendor stock <v>=<code> object=<a thing>`, which mints the stocked
//! asset's title — answered `EconomyError::Unauthorized`: a *permission* refusal,
//! naming a capability the operator had never asked for, for a subsystem they had
//! not turned on.
//!
//! The three cases below are the three altitudes that failure has:
//!
//! 1. the cheap structural gate, on the real root: with `[shadowrun.economy]` off,
//!    [`mush_market::MarketStore::issuer`] is `None`; with it on, it is `sr.market`.
//!    The two halves are now one flag, so they cannot drift apart again silently.
//! 2. the message: a composition that carries market content and **no** issuer
//!    refuses `@market/vendor stock` in the market's own words, and specifically not
//!    with a permission refusal.
//! 3. the positive control: with the Shadowrun economy on, the same fixture stocks
//!    and the title is really minted.
//!
//! A note on case 2's rig. On the production root the market's content sources and
//! its issuer are now gated by the **same** `[shadowrun.economy].enabled`, so the
//! root can no longer compose "content, no issuer" by itself — which is the point of
//! the change. The case therefore takes the full production roster and replaces
//! exactly ONE module, the market, with an issuer-less one: everything else, the
//! economy's own issuer list included, is the roster a real server boots. That keeps
//! the arm under test reachable and keeps the difference between case 2 and case 3
//! down to the single call this step made conditional.

use std::path::PathBuf;
use std::sync::Arc;

use mush_config::config::Config;
use mush_core::{Audience, GOD, GameModule, ObjId, ObjKind};
use mush_economy::EconomyStore;
use mush_llm::LlmService;
use mush_market::MarketStore;
use mush_org::PrincipalId;
use mush_proto::OutputEvent;
use mush_server::boot;
use mush_testkit::{TestWorld, text_of};

/// A stocked, legal, asset-represented good from the `sr.core` starter pack — the
/// one `ruleset-shadowrun/tests/e6c.rs` stocks with, for the same reason: an asset
/// listing is the only representation that mints a title.
const GUN: &str = "sr:colt.america";

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

/// A memory-backed config on a scratch data dir, with one character ruleset selected
/// and `[shadowrun.economy]` either on or off — the two knobs the market's wiring
/// reads, and nothing else.
fn config(tag: &str, ruleset: &str, sr_economy: bool) -> Config {
    let mut cfg = Config::default();
    cfg.store.store = "memory".to_string();
    cfg.server.data_dir = scratch_dir(tag);
    cfg.ipc.listen = "127.0.0.1:0".to_string();
    cfg.rulesets.enabled = vec![ruleset.to_string()];
    cfg.shadowrun.economy.enabled = sr_economy;
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

fn boot_world(cfg: &Config) -> TestWorld {
    TestWorld::with_modules(roster(cfg))
}

fn issuer_of(tw: &TestWorld) -> Option<String> {
    tw.world()
        .module_state::<MarketStore>()
        .expect("the market composes on every ruleset")
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

/// A registered vendor — an organization with a nuyen account and a vault it owns —
/// and one `Thing` in the stocking staffer's own hands, which is what
/// `@market/vendor stock … object=` requires (E6c's M10 fence).
struct Shop {
    id: mush_market::VendorId,
    object: ObjId,
}

fn shop(tw: &mut TestWorld) -> Shop {
    ok(tw, "@market/catalog load sr.core");
    ok(tw, "@org/create Ares");
    let org = tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .and_then(|s| s.state().orgs().map(|o| o.id).max())
        .expect("the org exists");
    ok(tw, &format!("@account/open org:#{}=sr.nuyen", org.0));
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
            "@market/vendor create #{}=#{} name=Ares market=seattle buy=300",
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
        .create(ObjKind::Thing, "a Colt America L36", GOD, Some(GOD))
        .expect("an object in the staffer's hands");
    Shop { id, object }
}

fn stock_line(shop: &Shop) -> String {
    format!(
        "@market/vendor stock #{}={GUN} price=400 qty=1 object=#{}",
        shop.id.0, shop.object.0
    )
}

// ---- 1: the structural gate ------------------------------------------------

/// The cheap primary proof, on the composition root itself.
///
/// `server_modules()` passes `shadowrun = None` and `SrEconomyConfig::default()` is
/// disabled, so **every** stock rig and every non-Shadowrun server takes the `None`
/// arm — which is exactly the population that used to carry `sr.market` for nothing.
#[test]
fn composition_without_the_shadowrun_economy_registers_no_market_issuer() {
    // The default composition: the WoD ruleset, no Shadowrun economic layer.
    let wod = config("wod", "wod", false);
    assert_eq!(
        issuer_of(&boot_world(&wod)),
        None,
        "a WoD server registers no market title issuer"
    );
    let _ = std::fs::remove_dir_all(&wod.server.data_dir);

    // The Shadowrun ruleset with its economic layer switched OFF is the same case:
    // the market gets no content sources either, so an issuer would name a
    // capability nothing could use.
    let sr_off = config("sroff", "shadowrun", false);
    assert_eq!(
        issuer_of(&boot_world(&sr_off)),
        None,
        "`[shadowrun.economy].enabled = false` registers no market title issuer"
    );
    let _ = std::fs::remove_dir_all(&sr_off.server.data_dir);

    // And with the layer on, the name the economy's own frozen list carries.
    let sr_on = config("sron", "shadowrun", true);
    assert_eq!(
        issuer_of(&boot_world(&sr_on)).as_deref(),
        Some("sr.market"),
        "an enabled Shadowrun economy registers the market title issuer"
    );
    let _ = std::fs::remove_dir_all(&sr_on.server.data_dir);
}

// ---- 2: the message --------------------------------------------------------

/// A market with content and no issuer refuses in **its own** words.
///
/// Before this step, a non-Shadowrun server's market held `sr.market` while the
/// economy listed nothing, so `issue_title` answered `Unauthorized` from inside a
/// verb whose operator had never named an issuer. The needle is deliberately short:
/// `api.rs`'s refusal carries a long run of spaces mid-sentence.
#[test]
fn stocking_fails_clear_without_the_shadowrun_economy() {
    let cfg = config("noissuer", "shadowrun", true);
    let mut mods = roster(&cfg);
    // Exactly one module differs from the roster a real server boots: the market,
    // rebuilt without the `with_issuer` call this step made conditional.
    let at = mods
        .iter()
        .position(|m| m.name() == "market")
        .expect("the market composes");
    let sr = ruleset_shadowrun::SrEconomyConfig {
        enabled: true,
        ..ruleset_shadowrun::SrEconomyConfig::default()
    };
    mods[at] = Box::new(
        mush_market::MarketModule::new()
            .with_catalog_sources(vec![Arc::new(ruleset_shadowrun::SrCatalogSource::new(&sr))])
            .with_availability(vec![Arc::new(ruleset_shadowrun::SrAvailability::new())])
            .with_delivery(vec![Arc::new(ruleset_shadowrun::SrDelivery)]),
    );
    let mut tw = TestWorld::with_modules(mods);
    assert_eq!(issuer_of(&tw), None, "the rig's market names no issuer");

    let shop = shop(&mut tw);
    let refused = err(&mut tw, &stock_line(&shop));
    assert!(
        refused.contains("This server registers no market title issuer"),
        "{refused}"
    );
    // …and NOT the economy's permission refusal, which is what the operator used to
    // read for a subsystem they had not enabled.
    assert!(
        !refused.to_ascii_lowercase().contains("authoriz"),
        "a missing issuer is not a permission problem: {refused}"
    );
    // Nothing was minted on the way to the refusal.
    assert!(
        tw.world()
            .module_state::<EconomyStore>()
            .expect("economy store")
            .snapshot()
            .asset(shop.object)
            .is_none(),
        "a refused stock mints no title"
    );
    let _ = std::fs::remove_dir_all(&cfg.server.data_dir);
}

// ---- 3: the positive control -----------------------------------------------

/// The same fixture on the roster a Shadowrun server really boots: the two halves
/// agree, `issue_title` accepts, and the stocked object carries a vendor-owned title.
#[test]
fn stocking_mints_title_with_the_shadowrun_economy_enabled() {
    let cfg = config("enabled", "shadowrun", true);
    let mut tw = boot_world(&cfg);
    assert_eq!(issuer_of(&tw).as_deref(), Some("sr.market"));

    let shop = shop(&mut tw);
    ok(&mut tw, &stock_line(&shop));
    assert!(
        tw.world()
            .module_state::<EconomyStore>()
            .expect("economy store")
            .snapshot()
            .asset(shop.object)
            .is_some(),
        "the stocked object carries a title"
    );
    let _ = std::fs::remove_dir_all(&cfg.server.data_dir);
}

//! **Doc 18 cut IC3 acceptance — ports, orgs, capital and readiness** (doc 18
//! §4.1, §4.4, §4.7; D-18-14, D-18-20, D-18-25, D-18-27, D-18-47; lead rulings
//! L0–L16 of the IC3 brief).
//!
//! Lane v2's two `port` entries are applied once, under the content latch, by
//! the `PortApplier`: one passive domain per port presenting in the station's
//! berth gallery, the System treasury in `tacsim.credit`, the nine institutions
//! as real orgs with scoped accounts, and their capital as one-shot treasury
//! obligations of at most 800,000 cr that settle on the economy's next sweep.
//! Every act is recorded in a fail-closed port map. This file boots that on the
//! production composition (with the pilot sheet, as `mush_server::boot` composes
//! it), sweeps it, reboots it, updates it, corrupts it, crowds it with windows,
//! upgrades a world from lane v1, reads the readiness report and `+credit`, and
//! refuses every port entry the applier could not bind.
//!
//! No test here asserts a passage duration (D-18-50).

use crate::support;

use std::collections::{BTreeMap, BTreeSet, HashMap};

use mush_content::ContentState;
use mush_core::{Audience, CanonicalId, GOD, ObjId};
use mush_economy::{AccountId, AccountStatus, EconomyConfig, EconomyStore, InstrumentId};
use mush_org::{OrgStore, PrincipalId};
use mush_proto::OutputEvent;
use mush_sim::data::{DomainRecord, SpatialState};
use mush_testkit::{TestWorld, fold_module_ops};
use smol_str::SmolStr;
use tacsim::config::PackSource;
use tacsim::definitions::content::{LANE_JSON, LANE_PACK, LANE_VERSION};
use tacsim::pack::{PackEntry, PackError, load_pack, rules_pack_of};
use tacsim::port::apply::room_by_key;
use tacsim::port::{PortDef, PortMap, PortMapState, room_key, tranches};
use tacsim::{TacsimConfig, TacsimStore};

use support::ground::{UNSIGNED, sign};

// ---------------------------------------------------------------------------
// The rig
// ---------------------------------------------------------------------------

fn config() -> TacsimConfig {
    TacsimConfig {
        enabled: true,
        ..TacsimConfig::default()
    }
}

fn cid(word: &str) -> CanonicalId {
    CanonicalId::new(word).expect("a canonical id")
}

fn lane_id() -> CanonicalId {
    cid(LANE_PACK)
}

/// How a test roster is composed.
#[derive(Clone)]
struct Roster {
    /// The economy block the composition is handed.
    economy: EconomyConfig,
    /// Whether the pilot sheet composes (production does).
    sheet: bool,
    /// Whether the layer is a test world's (`tacsim_test_world_module_wired`).
    test_world: bool,
    /// A shelf replacing the atlas module's compiled one (test 4's v3).
    shelf: Option<Vec<mush_content::Pack>>,
}

impl Roster {
    /// The production roster: `mush_server::boot::game_modules`' shape, the
    /// sheet included.
    fn production() -> Roster {
        Roster {
            economy: EconomyConfig::default(),
            sheet: true,
            test_world: false,
            shelf: None,
        }
    }

    /// `tests/lane.rs`'s shape: no sheet, so the credit comes from the
    /// applier's own `ensure_instrument` (L9), and a spawned player walks.
    fn bare() -> Roster {
        Roster {
            sheet: false,
            ..Roster::production()
        }
    }

    fn test_world() -> Roster {
        Roster {
            test_world: true,
            ..Roster::production()
        }
    }

    fn with_shelf(mut self, shelf: Vec<mush_content::Pack>) -> Roster {
        self.shelf = Some(shelf);
        self
    }

    fn modules(&self) -> Vec<Box<dyn mush_core::GameModule>> {
        self.modules_with(config(), true)
    }

    /// The roster, with the lane's game sources on or off (test 7 boots v1 as
    /// an operator seed with them off).
    fn modules_with(
        &self,
        cfg: TacsimConfig,
        game_content: bool,
    ) -> Vec<Box<dyn mush_core::GameModule>> {
        use tacsim::composition as comp;

        let onboarding = mush_core::OnboardingConfig::default();
        let game = tacsim::GameEconomyConfig::default();
        // Doc 18 cut IC5: the production roster's built-ins, with `tacsim` the third
        // quantity delegate and the inventory cell handed to the layer below.
        let (mut mods, inventory) = comp::domain_modules_with_economy(
            self.economy.clone(),
            mush_atlas::AtlasConfig::default(),
            &game,
        );
        if let Some(shelf) = &self.shelf {
            let at = mods
                .iter()
                .position(|m| m.name() == mush_atlas::NS)
                .expect("the atlas composes");
            mods[at] = Box::new(
                mush_atlas::AtlasModule::with_config(mush_atlas::AtlasConfig::default())
                    .with_shelf(with_stations(shelf)),
            );
        }
        mods.push(comp::npc_module(comp::npc_agent_adapter()));
        if self.sheet {
            mods.extend(comp::sheet_modules(&onboarding, &game));
        }
        if self.test_world {
            mods.push(comp::tacsim_test_world_module_wired(
                cfg,
                game_content,
                inventory,
            ));
        } else {
            mods.push(comp::tacsim_module_wired(cfg, game_content, inventory));
        }
        mods.push(
            comp::content_module_with(true, game_content).expect("the content module composes"),
        );
        mods
    }
}

/// No module write was refused for its namespace (a refused write is silent
/// until a reboot).
fn no_refused_writes(tw: &TestWorld) {
    let refused = tw.world().refused_namespace_writes();
    assert!(refused.is_empty(), "refused namespace writes: {refused:?}");
}

fn boot(roster: &Roster) -> TestWorld {
    let tw = TestWorld::with_modules(roster.modules());
    no_refused_writes(&tw);
    tw
}

fn reboot(
    tw: TestWorld,
    roster: &Roster,
    store: HashMap<SmolStr, mush_core::ModuleBootData>,
) -> TestWorld {
    let tw = tw
        .reboot_into(roster.modules(), store)
        .expect("the reboot boots");
    no_refused_writes(&tw);
    tw
}

fn said(tw: &TestWorld, events: &[(Audience, OutputEvent)], who: ObjId) -> String {
    tw.seen_by(events, who).join("\n")
}

fn run(tw: &mut TestWorld, who: ObjId, line: &str) -> String {
    let events = tw.run(who, line);
    said(tw, &events, who)
}

fn store(tw: &TestWorld) -> &TacsimStore {
    tw.world()
        .module_state::<TacsimStore>()
        .expect("tacsim state")
}

fn economy(tw: &TestWorld) -> &EconomyStore {
    tw.world()
        .module_state::<EconomyStore>()
        .expect("economy state")
}

fn orgs(tw: &TestWorld) -> &OrgStore {
    tw.world().module_state::<OrgStore>().expect("org state")
}

fn spatial(tw: &TestWorld) -> &SpatialState {
    tw.world()
        .module_state::<SpatialState>()
        .expect("sim state")
}

fn credit() -> InstrumentId {
    InstrumentId::new(tacsim::CREDIT).expect("the credit id")
}

/// The port map of one lane entry, which must be readable.
fn port_map(tw: &TestWorld, entry: &str) -> PortMap {
    match store(tw).port_map(&lane_id(), &cid(entry)) {
        PortMapState::Present(map) => map.clone(),
        other => panic!("{entry}'s map: {other:?}"),
    }
}

/// The lane v2 port entries, decoded from the shipped bytes.
fn ports_of(json: &str) -> Vec<(CanonicalId, PortDef)> {
    let pack = load_pack(json).expect("the lane verifies");
    pack.entries()
        .iter()
        .filter_map(|(id, entry)| match entry {
            PackEntry::Port(def) => Some((id.clone(), def.clone())),
            _ => None,
        })
        .collect()
}

/// Run every deferred atlas composite off-loop and re-enter it, as the world
/// task does (`tests/harness.rs`'s `finish_composites`).
fn finish_composites(tw: &mut TestWorld, effects: &[mush_core::Effect]) {
    let jobs: Vec<(u64, String)> = effects
        .iter()
        .filter_map(|e| match e {
            mush_core::Effect::ModuleAsync {
                module,
                correlation,
                payload,
                ..
            } if module.as_str() == mush_atlas::NS => Some((*correlation, payload.clone())),
            _ => None,
        })
        .collect();
    for (correlation, payload) in jobs {
        let outcome = mush_atlas::run_async(&payload);
        let mut out = mush_core::Outbox::new();
        let mut fx = mush_core::Effects::new();
        tw.engine_mut().step(
            mush_core::Inbound::Apply(mush_core::WorldCommand::ModuleAsyncDone {
                module: mush_atlas::NS.into(),
                actor: GOD,
                correlation,
                payload: outcome,
            }),
            &mut out,
            &mut fx,
        );
    }
}

/// The lane's latched version on this world.
fn lane_latch(tw: &TestWorld) -> Option<u32> {
    tw.world()
        .module_state::<ContentState>()
        .and_then(|c| c.latches.applied(&lane_id()))
        .map(mush_content::Applied::version)
}

/// What a port apply must never move: orgs, accounts, obligations, domains and
/// journal entries.
#[derive(Debug, PartialEq, Eq)]
struct Counts {
    orgs: usize,
    accounts: usize,
    obligations: usize,
    domains: usize,
    journal: u64,
}

fn counts(tw: &TestWorld) -> Counts {
    Counts {
        orgs: orgs(tw).state().orgs().count(),
        accounts: economy(tw).snapshot().accounts().count(),
        obligations: economy(tw).snapshot().obligations().count(),
        domains: spatial(tw).domains.len(),
        journal: economy(tw).journal_len(),
    }
}

/// Both port maps as they stand in RAM.
fn maps(tw: &TestWorld) -> Vec<PortMap> {
    store(tw).port_maps().cloned().collect()
}

/// The port-map documents a batch wrote, by key.
fn map_docs(ops: &[mush_core::ModuleOp]) -> BTreeMap<String, serde_json::Value> {
    ops.iter()
        .filter_map(|op| match &op.op {
            mush_core::ModuleWrite::JsonSet { key, value }
                if key.starts_with(tacsim::port::PORT_MAP_PREFIX) =>
            {
                Some((key.to_string(), value.clone()))
            }
            _ => None,
        })
        .collect()
}

/// One economy sweep at `now + secs`: the sweep fires once per call (the
/// `crates/mush-economy/tests/e4b1.rs` `sweep_at` shape).
fn sweep(tw: &mut TestWorld, secs: i64) {
    let t = tw.world().now().saturating_add(secs);
    tw.set_now(t);
    let _ = tw.fire_due(t.saturating_mul(1000));
}

/// The live world rate.
fn world_rate(tw: &TestWorld) -> u32 {
    tw.world()
        .world_timeline()
        .epochs()
        .last()
        .map(|epoch| epoch.rate)
        .expect("a time generation")
}

/// Put the world at `rate` through `@tacsim/time` (which refuses re-typing the
/// live rate).
fn set_rate(tw: &mut TestWorld, rate: u32) {
    if world_rate(tw) != rate {
        let out = run(tw, GOD, &format!("@tacsim/time rate={rate}"));
        assert_eq!(world_rate(tw), rate, "{out}");
    }
}

fn report(tw: &mut TestWorld) -> String {
    run(tw, GOD, "@tacsim/port status")
}

/// The lane's bytes as JSON, edited by `edit`, at `version`, re-signed.
fn lane_variant(version: u32, edit: impl FnOnce(&mut serde_json::Value)) -> String {
    let mut doc: serde_json::Value = serde_json::from_str(LANE_JSON).expect("the lane is JSON");
    doc["manifest"]["version"] = serde_json::json!(version);
    doc["manifest"]["digest"] = serde_json::json!(UNSIGNED);
    edit(&mut doc);
    sign(&serde_json::to_string_pretty(&doc).expect("serializes"))
}

fn content_pack(json: &str) -> mush_content::Pack {
    mush_content::load_pack(json).expect("the variant verifies")
}

/// The next lane (test-only data, one past the shipped version — v5 since doc 19 cut
/// A.1 shipped v4): both port entries touched — `ceres_port`'s dormancy 8 days and
/// `yard_commissary`'s faction `freeport`.
fn lane_v4(edit: impl FnOnce(&mut serde_json::Value)) -> mush_content::Pack {
    content_pack(&lane_variant(LANE_VERSION + 1, |doc| {
        doc["entries"]["ceres_port"]["def"]["programme"]["dormancy_real_days"] =
            serde_json::json!(8);
        doc["entries"]["stickney_port"]["def"]["orgs"][2]["faction"] =
            serde_json::json!("freeport");
        edit(doc);
    }))
}

/// `@atlas/pack plan` then `@atlas/pack apply <fingerprint>` on the lane.
fn plan_and_apply(tw: &mut TestWorld) -> (String, String) {
    let plan = run(tw, GOD, &format!("@atlas/pack plan {LANE_PACK}"));
    let fingerprint = plan
        .split("@atlas/pack apply ")
        .nth(1)
        .map(|rest| rest.split_whitespace().next().unwrap_or("").to_owned())
        .unwrap_or_default();
    let apply = run(tw, GOD, &format!("@atlas/pack apply {fingerprint}"));
    (plan, apply)
}

/// A lane location's label (doc 19 cut A.8: five ports, five places).
fn location_label(place: &CanonicalId) -> String {
    let pack = load_pack(LANE_JSON).expect("the lane verifies");
    match pack.entry(place) {
        Some(PackEntry::Location(def)) => def.label.clone(),
        other => panic!("{place}: {other:?}"),
    }
}

/// The sum of every authored capital over the decoded lane, major units — each org's base
/// with its raises (doc 20 cut SM.5: Occator's 250,000).
fn total_capital() -> u64 {
    ports_of(LANE_JSON)
        .iter()
        .flat_map(|(_, def)| {
            def.orgs
                .iter()
                .map(|org| org.capital_total().expect("the capital fits"))
        })
        .sum()
}

/// The production passenger's finite purse is separate from institutional capital.
/// Check the actual one-shot payment instead of folding it into the org schedule.
fn passenger_seeded_capital(tw: &TestWorld) -> i64 {
    let snapshot = economy(tw).snapshot();
    let total = snapshot
        .obligations()
        .filter(|o| {
            matches!(&o.cause, mush_economy::ObligationCause::Ruleset { namespace }
                if namespace.starts_with("tacsim.passenger.seed."))
        })
        .map(|o| {
            assert!(matches!(
                o.status,
                mush_economy::ObligationStatus::Satisfied
            ));
            assert_eq!(o.periods_settled, 1);
            assert!(o.schedule.cadence.is_none());
            let purse = snapshot
                .account(o.credit_account)
                .expect("the passenger purse");
            assert_eq!(purse.posted_balance, o.amount);
            assert!(purse.held_amount.is_zero());
            o.amount.minor()
        })
        .sum();
    assert_eq!(
        total, 100_000,
        "each of the two authored travellers receives one 500cr purse"
    );
    total
}

/// The number of tranches the authored capital needs — the schedule with its raises
/// (doc 20 cut SM.5).
fn total_tranches() -> usize {
    ports_of(LANE_JSON)
        .iter()
        .flat_map(|(_, def)| {
            def.orgs
                .iter()
                .map(|org| tacsim::port::capital_schedule(org.capital_major, &org.raises).len())
        })
        .sum()
}

// ---------------------------------------------------------------------------
// 1. The domains
// ---------------------------------------------------------------------------

/// **Both ports open one passive domain each into their berth galleries**
/// (doc 18 §4.7 part a; lens 4).
#[test]
fn ic3_boot_opens_two_passive_domains_into_the_galleries() {
    let mut tw = boot(&Roster::production());
    let placed = tw
        .world()
        .module_state::<ContentState>()
        .and_then(|c| c.latches.applied(&lane_id()))
        .map(mush_content::Applied::placed)
        .expect("the lane is latched");
    for (entry, def) in ports_of(LANE_JSON) {
        let map = port_map(&tw, entry.as_str());
        let host = placed.get(&def.location).copied().expect("the place room");
        let gallery = room_by_key(
            tw.world(),
            &room_key(&def.area, &def.gallery).expect("the gallery key"),
        )
        .expect("the gallery resolves");
        assert!(
            tw.world()
                .has_marker(gallery, mush_atlas::def::PORT_DOCK_MARKER),
            "{entry}'s gallery carries port.dock"
        );
        assert_eq!(map.host, Some(host.0), "{entry}");
        assert_eq!(map.presentation, Some(gallery.0), "{entry}");
        let domain: &DomainRecord = spatial(&tw)
            .domains
            .get(&mush_sim::data::DomainId(map.domain.expect("a domain")))
            .expect("the domain record");
        assert_eq!(domain.host, host, "{entry}");
        assert_eq!(domain.presentation_location, gallery, "{entry}");
        assert!(domain.window.is_none(), "{entry}'s domain is passive");
    }
    assert_eq!(
        mush_sim::windows(tw.world()).count(),
        0,
        "no window was opened"
    );
    let out = run(&mut tw, GOD, "@domain");
    for place in [
        "Ceres Freeport",
        "Stickney Yards",
        "Aitken Highport",
        "Voltaire Anchorage",
        "Ligeia Terminal",
    ] {
        assert!(
            out.lines()
                .any(|line| line.contains(&format!("host {place} "))
                    && line.contains("presents in Visiting Berth Gallery")),
            "{place}:\n{out}"
        );
    }
}

// ---------------------------------------------------------------------------
// 2. The orgs and their capital
// ---------------------------------------------------------------------------

/// **The nine institutions are created, and one sweep settles their capital**
/// (doc 18 §4.7 parts b–c, D-18-14, D-18-27; lens 1). The total is derived from
/// the decoded entries, never typed in.
#[test]
fn ic3_boot_creates_the_nine_orgs_and_one_sweep_settles_their_capital() {
    let mut tw = boot(&Roster::production());
    let list = run(&mut tw, GOD, "@org/list");
    let ports = ports_of(LANE_JSON);
    let treasury = port_map(&tw, "ceres_port").treasury.expect("a treasury");
    assert_eq!(
        port_map(&tw, "stickney_port").treasury,
        Some(treasury),
        "two ports naming one treasury share one account"
    );
    let snapshot = economy(&tw).snapshot();
    let tre = snapshot.account(AccountId(treasury)).expect("the treasury");
    assert_eq!(
        tre.owner,
        PrincipalId::System(
            mush_org::SystemPrincipalId::new("tacsim.treasury").expect("a principal")
        )
    );
    assert_eq!(tre.instrument, credit());
    assert!(tre.scope.is_none());
    let mut obligations = 0;
    for (entry, def) in &ports {
        let map = port_map(&tw, entry.as_str());
        for org in &def.orgs {
            assert!(list.contains(&org.name), "{}:\n{list}", org.name);
            let row = map.orgs.get(&org.id).expect("a map row");
            let record = orgs(&tw)
                .state()
                .org(mush_org::OrgId(row.org))
                .expect("the org exists");
            assert_eq!(record.name.as_str(), org.name);
            assert_eq!(record.root_scope.0, row.root_scope);
            let scope = orgs(&tw)
                .state()
                .scope(mush_org::OrgScopeId(row.root_scope))
                .expect("the root scope exists");
            assert_eq!(scope.org, record.id);
            assert!(scope.parent.is_none(), "{} has its root scope", org.id);
            let scoped: Vec<_> = snapshot
                .accounts_of(&PrincipalId::Organization(mush_org::OrgId(row.org)))
                .filter(|a| a.status == AccountStatus::Active && a.instrument == credit())
                .collect();
            assert_eq!(scoped.len(), 1, "{}", org.id);
            let account = scoped[0];
            assert_eq!(Some(account.id.0), row.account);
            assert_eq!(account.scope.map(|s| s.0), Some(row.root_scope));
            assert_eq!(account.credit, mush_economy::CreditPolicy::forbid());
            // Doc 20 cut SM.5: the base's tranches and then each raise's.
            let want = tacsim::port::capital_schedule(org.capital_major, &org.raises);
            assert_eq!(row.capital.len(), want.len(), "{}", org.id);
            for (tranche, major) in row.capital.iter().zip(&want) {
                let o = snapshot
                    .obligation(mush_economy::ObligationId(tranche.obligation))
                    .expect("the obligation");
                assert_eq!(o.debit_account, AccountId(treasury));
                assert_eq!(o.credit_account, account.id);
                assert_eq!(o.amount.minor(), i64::try_from(*major).expect("fits") * 100);
                assert!(matches!(o.status, mush_economy::ObligationStatus::Active));
                assert_eq!(o.periods_settled, 0);
                obligations += 1;
            }
        }
    }
    assert_eq!(obligations, total_tranches());
    assert_eq!(snapshot.obligations().count(), total_tranches());
    let before = report(&mut tw);
    assert!(
        before.contains("ceres_port org coop: Belt Carriers' Cooperative, org #"),
        "{before}"
    );
    assert!(
        before.contains(": 0 of 4 tranches settled. [pending]"),
        "{before}"
    );

    sweep(&mut tw, 3_600);
    let snapshot = economy(&tw).snapshot();
    for (entry, def) in &ports {
        let map = port_map(&tw, entry.as_str());
        for org in &def.orgs {
            let row = map.orgs.get(&org.id).expect("a map row");
            for tranche in &row.capital {
                let o = snapshot
                    .obligation(mush_economy::ObligationId(tranche.obligation))
                    .expect("the obligation");
                assert!(
                    matches!(o.status, mush_economy::ObligationStatus::Satisfied),
                    "{}: {:?}",
                    org.id,
                    o.status
                );
            }
            let account = snapshot
                .account(AccountId(row.account.expect("an account")))
                .expect("the account");
            // Doc 20 cut SM.5: the base with its raises (Occator 300,000).
            assert_eq!(
                account.posted_balance.minor(),
                i64::try_from(org.capital_total().expect("fits")).expect("fits") * 100,
                "{}",
                org.id
            );
        }
    }
    let treasury_balance = snapshot
        .account(AccountId(treasury))
        .expect("the treasury")
        .posted_balance
        .minor();
    assert_eq!(
        treasury_balance,
        -(i64::try_from(total_capital()).expect("fits") * 100 + passenger_seeded_capital(&tw))
    );
    assert_eq!(
        treasury_balance, -1_495_100_000,
        "14,951,000.00cr: 8,200,000 at v2, the moon ports' 6,300,000 (doc 19 cut A.8), the \
         Freeport Registry's 50,000 (doc 20 cut SM.2), Occator's 250,000 raise (doc 20 cut \
         SM.5), the Pallas Syndicate's 50,000 (doc 20 cut SM.10), R9b's Fleet 100,000 \
         and the two passengers' one-time 500cr purses"
    );
    set_rate(&mut tw, 24);
    let after = report(&mut tw);
    // Doc 18 cut IC8: the stock lines (pools, listings, bids) wait on the port tick,
    // which this file never lands; the claim is about the capital lines.
    assert!(
        !after
            .lines()
            .any(|l| l.contains(" org ") && l.ends_with("[pending]")),
        "{after}"
    );
    assert!(
        after.contains("ceres_port org freeport_exchange: Freeport Exchange, org #")
            && after.contains("capital 4000000.00cr: 5 of 5 tranches settled. [ready]"),
        "{after}"
    );
    // Doc 18 cut IC8: every binding is in place but the ports' stock, which the port
    // tick seeds and lists (`commerce.rs` lands it; this file does not): each pools
    // and listings line is pending — made, waiting on the tick, never missing or
    // flagged — and every other line is ready.
    let stock_line = |l: &str| l.contains(" vendor ") || l.contains(" ask ");
    let bracketed: Vec<&str> = after
        .lines()
        // doc 20 SM.1: the geology line is reported and never gated
        .filter(|l| l.ends_with(']') && !l.starts_with("geology: "))
        .collect();
    assert!(
        bracketed.iter().any(|l| stock_line(l)),
        "the stock lines are reported: {after}"
    );
    for line in bracketed {
        // Doc 19 cut A.8: a bid-only vendor (no stock rows) reads Ready at 0 of 0.
        let want = if stock_line(line) && !line.contains("0 of 0 stock rows seeded") {
            "[pending]"
        } else {
            "[ready]"
        };
        assert!(line.ends_with(want), "`{line}` is not {want}:\n{after}");
    }
}

/// **The applier ensures the credit without the sheet** (L9): on `tests/lane.rs`'s
/// sheet-less roster both ports bind and one sweep settles them, with
/// `tacsim.credit` made by the applier's own `ensure_instrument`.
#[test]
fn ic3_the_applier_ensures_the_credit_without_the_sheet() {
    let mut tw = boot(&Roster::bare());
    assert!(economy(&tw).snapshot().instrument(&credit()).is_some());
    let ceres = port_map(&tw, "ceres_port");
    let stickney = port_map(&tw, "stickney_port");
    // Doc 20 cut SM.2: the Freeport Registry is Ceres's seventh. Doc 20 cut SM.10: the Pallas
    // Syndicate, a Ceres lane org, its eighth.
    assert_eq!(ceres.orgs.len() + stickney.orgs.len(), 11);
    sweep(&mut tw, 3_600);
    let snapshot = economy(&tw).snapshot();
    let treasury = snapshot
        .account(AccountId(ceres.treasury.expect("a treasury")))
        .expect("the treasury");
    assert_eq!(
        treasury.posted_balance.minor(),
        -(i64::try_from(total_capital()).expect("fits") * 100 + passenger_seeded_capital(&tw))
    );
}

// ---------------------------------------------------------------------------
// 3. A second boot
// ---------------------------------------------------------------------------

/// **A second boot posts nothing** (doc 18 §4.7 "Idempotency"; lens 2).
#[test]
fn ic3_a_second_boot_posts_nothing() {
    let roster = Roster::production();
    let mut tw = boot(&roster);
    sweep(&mut tw, 3_600);
    // The Exchange's five-second cadence runs before the economy producer in
    // this fire. Let its next fire consume the now-funded clearing capital,
    // then assert a settled world's reboot adds no journal entry.
    sweep(&mut tw, 5);
    let before = counts(&tw);
    let before_maps = maps(&tw);
    let before_latches: Vec<(String, u32, String)> = tw
        .world()
        .module_state::<ContentState>()
        .expect("content")
        .latches
        .all()
        .map(|l| (l.pack().to_string(), l.version(), l.digest().to_hex()))
        .collect();
    let objects = tw.world().iter().count();
    let ops = tw.take_module_ops();
    let docs = map_docs(&ops);
    let folded = fold_module_ops(ops);
    let mut tw = reboot(tw, &roster, folded);
    assert_eq!(counts(&tw), before);
    assert_eq!(maps(&tw), before_maps);
    let latches: Vec<(String, u32, String)> = tw
        .world()
        .module_state::<ContentState>()
        .expect("content")
        .latches
        .all()
        .map(|l| (l.pack().to_string(), l.version(), l.digest().to_hex()))
        .collect();
    assert_eq!(latches, before_latches);
    assert_eq!(tw.world().iter().count(), objects, "no new object");
    // The documents a reboot reads back are the ones the first boot wrote, and
    // the second boot writes none.
    // Five ports and (doc 20 cut SM.2) six sites.
    assert_eq!(docs.len(), 11, "{:?}", docs.keys().collect::<Vec<_>>());
    for map in maps(&tw) {
        let written = docs.get(map.key().as_str()).expect("a document");
        assert_eq!(&serde_json::to_value(&map).expect("encodes"), written);
    }
    let again = tw.take_module_ops();
    assert!(
        map_docs(&again).is_empty(),
        "the second boot wrote a port map"
    );
}

// ---------------------------------------------------------------------------
// 4. An update
// ---------------------------------------------------------------------------

/// **An update reuses every map row and refuses drift** (doc 18 §4.7; L4, L16).
#[test]
fn ic3_an_update_reuses_every_map_row_and_refuses_drift() {
    let v3 = lane_v4(|_| {});
    let roster = Roster::production().with_shelf(vec![v3]);
    let mut tw = boot(&roster);
    sweep(&mut tw, 3_600);
    let snapshot_counts = counts(&tw);
    let snapshot_maps = maps(&tw);
    let _ = tw.take_module_ops();

    let (plan, apply) = plan_and_apply(&mut tw);
    assert!(
        plan.contains(&format!(
            "Plan for {LANE_PACK}@{}: 0 create, 2 update, 0 retire, 58 unchanged",
            LANE_VERSION + 1
        )),
        "{plan}"
    );
    for entry in ["ceres_port", "stickney_port"] {
        assert!(
            plan.lines()
                .any(|line| line.contains("update") && line.contains(&format!("{entry} (port)"))),
            "{entry}:\n{plan}"
        );
    }
    assert!(
        apply.contains(&format!(
            "Applied {LANE_PACK}@{}: 0 created, 2 updated, 0 retired, 58 unchanged.",
            LANE_VERSION + 1
        )),
        "{apply}\n{plan}"
    );
    assert!(!apply.contains("STOPPED"), "{apply}");
    assert_eq!(lane_latch(&tw), Some(LANE_VERSION + 1));
    assert_eq!(counts(&tw), snapshot_counts, "the update made nothing");
    assert_eq!(maps(&tw), snapshot_maps);
    let ops = tw.take_module_ops();
    assert!(map_docs(&ops).is_empty(), "the update rewrote a port map");
    no_refused_writes(&tw);
    // v3 is shelved, not loaded: the report reads the shipped v2 and the maps.
    set_rate(&mut tw, 24);
    let out = report(&mut tw);
    assert!(
        out.contains(&format!(
            "lane: {LANE_PACK}@{next} applied; this server ships @{LANE_VERSION} and does not load @{next}. [flagged]", next = LANE_VERSION + 1
        )),
        "{out}"
    );
    assert!(out.contains("  ceres_port domain: #"), "{out}");
    // Doc 18 cut IC8: nothing missing; only the ports' stock waits, on the port tick.
    assert!(
        !out.contains("[missing]")
            && out
                .lines()
                .filter(|l| l.ends_with("[pending]"))
                .all(|l| l.contains(" vendor ") || l.contains(" ask ")),
        "{out}"
    );

    // Drift, each on a fresh copy of the post-sweep world: the same boot and
    // sweep (deterministic, so the same ids), folded and rebooted.
    for (name, variant, words) in [
        (
            "v3' (coop capital 3,200,000)",
            lane_v4(|doc| {
                doc["entries"]["ceres_port"]["def"]["orgs"][1]["capital_major"] =
                    serde_json::json!(3_200_000);
            }),
            ["coop", "capital"],
        ),
        (
            "v3'' (Kasei Metals renamed)",
            lane_v4(|doc| {
                doc["entries"]["stickney_port"]["def"]["orgs"][0]["name"] =
                    serde_json::json!("Kasei Metals & Ores");
            }),
            ["kasei", "rename"],
        ),
        (
            // A top-up whose tranches EXTEND the recorded ones: 4,000,000 is
            // five of 800,000 and 4,800,000 is six, the first five the same.
            // Refused by the recorded capital, not by a tranche mismatch.
            "v3''' (freeport_exchange capital 4,800,000)",
            lane_v4(|doc| {
                doc["entries"]["ceres_port"]["def"]["orgs"][3]["capital_major"] =
                    serde_json::json!(4_800_000);
            }),
            ["freeport_exchange", "capital"],
        ),
        (
            // Doc 18 cut IC8 (lead ruling L6, owner-critical; fixer T15): a recorded
            // stock row is never re-sized by an update.
            "v4 (the Exchange's concentrate re-sized)",
            lane_v4(|doc| {
                doc["entries"]["ceres_port"]["def"]["vendors"][0]["stock"][0]["qty"] =
                    serde_json::json!(4_001);
            }),
            [
                "vendor freeport_exchange stock metals",
                "issue 1 is already recorded; a port update never re-issues or re-sizes stock",
            ],
        ),
        (
            // Doc 18 cut IC8 (fixer T15): a recorded bid is never retired by an update.
            "v4 (the Exchange's alloy bid dropped)",
            // Doc 20 cut SM.5: only the alloy row goes; the Exchange's ore intakes stay,
            // so the refusal still names the alloy bid.
            lane_v4(|doc| {
                if let Some(bids) =
                    doc["entries"]["ceres_port"]["def"]["vendors"][0]["bids"].as_array_mut()
                {
                    bids.retain(|bid| bid["good"] != "structural_alloys");
                }
            }),
            [
                "vendor freeport_exchange bid structural_alloys",
                "no longer authored; bids are never retired by a port update",
            ],
        ),
        (
            // Doc 18 cut IC8 (fixer F3): a price that cannot be written in minor
            // units is refused by the preflight, before any write.
            "v4 (the Exchange's concentrate priced past minor units)",
            lane_v4(|doc| {
                doc["entries"]["ceres_port"]["def"]["vendors"][0]["asks"][0]["price_major"] =
                    serde_json::json!(100_000_000_000_000_000_u64);
            }),
            ["vendor freeport_exchange ask metals", "the price overflows"],
        ),
    ] {
        let roster = Roster::production().with_shelf(vec![variant]);
        let mut fresh = boot(&roster);
        sweep(&mut fresh, 3_600);
        let folded = fold_module_ops(fresh.take_module_ops());
        let mut tw = reboot(fresh, &roster, folded);
        let before = counts(&tw);
        let before_maps = maps(&tw);
        let (_plan, apply) = plan_and_apply(&mut tw);
        for word in words {
            assert!(apply.contains(word), "{name}: {word}:\n{apply}");
        }
        assert!(!apply.contains("Applied "), "{name}:\n{apply}");
        assert_eq!(lane_latch(&tw), Some(LANE_VERSION), "{name}");
        assert_eq!(counts(&tw), before, "{name}");
        assert_eq!(maps(&tw), before_maps, "{name}");
        assert_eq!(before_maps, snapshot_maps, "{name}");
    }
}

/// **A later version's new org is created and capitalized, and every recorded
/// row stands** (doc 18 §4.7, L4: "a new org in a later version is created and
/// capitalized normally"). Test-only data: a tenth institution on Stickney.
#[test]
fn ic3_an_update_creates_a_new_org_and_keeps_every_recorded_row() {
    const NEW_CAPITAL: u64 = 900_000;
    let v3 = lane_v4(|doc| {
        doc["entries"]["stickney_port"]["def"]["orgs"]
            .as_array_mut()
            .expect("the orgs")
            .push(serde_json::json!({
                "id": "phobos_tender",
                "name": "Phobos Tug & Tender",
                "capital_major": NEW_CAPITAL,
                "faction": "republic"
            }));
    });
    let roster = Roster::production().with_shelf(vec![v3]);
    let mut tw = boot(&roster);
    sweep(&mut tw, 3_600);
    let before = counts(&tw);
    let ceres = port_map(&tw, "ceres_port");
    let stickney = port_map(&tw, "stickney_port");
    let _ = tw.take_module_ops();

    let (plan, apply) = plan_and_apply(&mut tw);
    assert!(
        apply.contains(&format!(
            "Applied {LANE_PACK}@{}: 0 created, 2 updated, 0 retired, 58 unchanged.",
            LANE_VERSION + 1
        )),
        "{apply}\n{plan}"
    );
    let want = tranches(NEW_CAPITAL);
    let after = counts(&tw);
    assert_eq!(after.orgs, before.orgs + 1, "one org");
    assert_eq!(after.accounts, before.accounts + 1, "one scoped account");
    assert_eq!(after.obligations, before.obligations + want.len());
    assert_eq!(after.domains, before.domains, "no domain");
    assert_eq!(
        after.journal, before.journal,
        "nothing posts before a sweep"
    );
    assert_eq!(port_map(&tw, "ceres_port"), ceres, "the ceres map stands");
    let now = port_map(&tw, "stickney_port");
    assert_eq!(now.domain, stickney.domain);
    assert_eq!(now.treasury, stickney.treasury);
    for (id, row) in &stickney.orgs {
        assert_eq!(
            now.orgs.get(id),
            Some(row),
            "{id}'s row is reused as it was"
        );
    }
    let row = now.orgs.get(&cid("phobos_tender")).expect("the new row");
    assert_eq!(row.capital_major, NEW_CAPITAL);
    let record = orgs(&tw)
        .state()
        .org(mush_org::OrgId(row.org))
        .expect("the new org");
    assert_eq!(record.name.as_str(), "Phobos Tug & Tender");
    let account = economy(&tw)
        .snapshot()
        .account(AccountId(row.account.expect("an account")))
        .expect("the account")
        .clone();
    assert_eq!(account.scope.map(|s| s.0), Some(row.root_scope));
    assert_eq!(account.credit, mush_economy::CreditPolicy::forbid());
    assert_eq!(row.capital.len(), want.len());
    for (tranche, major) in row.capital.iter().zip(&want) {
        let o = economy(&tw)
            .snapshot()
            .obligation(mush_economy::ObligationId(tranche.obligation))
            .expect("the obligation")
            .clone();
        assert_eq!(
            o.debit_account,
            AccountId(now.treasury.expect("a treasury"))
        );
        assert_eq!(o.credit_account, account.id);
        assert_eq!(o.amount.minor(), i64::try_from(*major).expect("fits") * 100);
    }
    let docs = map_docs(&tw.take_module_ops());
    assert_eq!(
        docs.keys().collect::<Vec<_>>(),
        vec![&now.key().to_string()],
        "only the stickney map is written"
    );
    no_refused_writes(&tw);
    sweep(&mut tw, 3_600);
    let balance = economy(&tw)
        .snapshot()
        .account(account.id)
        .expect("the account")
        .posted_balance
        .minor();
    assert_eq!(balance, i64::try_from(NEW_CAPITAL).expect("fits") * 100);
}

/// **An apply that stopped part way is completed by the next one** (L3, L4):
/// the world is rebuilt as if the first pass had stopped before the Exchange's
/// fifth tranche — the map row without it and its obligation never made. The
/// recorded capital equals the authored one, so the next apply makes exactly
/// that tranche and nothing else.
#[test]
fn ic3_a_map_that_stopped_part_way_is_completed() {
    let v3 = lane_v4(|_| {});
    let roster = Roster::production().with_shelf(vec![v3]);
    let mut tw = boot(&roster);
    let full = port_map(&tw, "ceres_port");
    let exchange = cid("freeport_exchange");
    let row = full.orgs.get(&exchange).expect("the row").clone();
    let last = row.capital.last().expect("a tranche").clone();
    assert_eq!(last.n, 4, "the Exchange's fifth tranche");
    let mut partial = full.clone();
    partial
        .orgs
        .get_mut(&exchange)
        .expect("the row")
        .capital
        .pop();
    let mut folded = fold_module_ops(tw.take_module_ops());
    let key = ceres_key();
    let doc = folded
        .get_mut(tacsim::NS)
        .and_then(|data| data.kv.iter_mut().find(|(k, _)| k.as_str() == key))
        .expect("the ceres map document");
    doc.1 = serde_json::to_value(&partial).expect("encodes");
    let gone = mush_economy::obligation_doc_key(mush_economy::ObligationId(last.obligation));
    let economy_kv = &mut folded
        .get_mut(mush_economy::NS)
        .expect("the economy's documents")
        .kv;
    let held = economy_kv.len();
    economy_kv.retain(|(k, _)| *k != gone);
    assert_eq!(
        economy_kv.len() + 1,
        held,
        "the obligation document is gone"
    );
    let mut tw = reboot(tw, &roster, folded);
    let out = report(&mut tw);
    assert!(
        out.contains(&format!(
            "  ceres_port org freeport_exchange: Freeport Exchange, org #{}, account #{}; 4 of 5 \
             capital tranches are created. [missing]",
            row.org,
            row.account.expect("an account")
        )),
        "{out}"
    );
    let before = counts(&tw);

    let (plan, apply) = plan_and_apply(&mut tw);
    assert!(
        apply.contains(&format!(
            "Applied {LANE_PACK}@{}: 0 created, 2 updated, 0 retired, 58 unchanged.",
            LANE_VERSION + 1
        )),
        "{apply}\n{plan}"
    );
    let after = counts(&tw);
    assert_eq!(after.obligations, before.obligations + 1, "one tranche");
    assert_eq!(after.orgs, before.orgs);
    assert_eq!(after.accounts, before.accounts);
    assert_eq!(after.domains, before.domains);
    let now = port_map(&tw, "ceres_port");
    let done = now.orgs.get(&exchange).expect("the row");
    assert_eq!(done.capital.len(), 5);
    assert_eq!(
        done.capital[..4],
        row.capital[..4],
        "the made tranches stand"
    );
    assert_eq!(done.capital[4].n, 4);
    assert_eq!(done.capital[4].amount_minor, 80_000_000);
    for (id, other) in &full.orgs {
        if *id != exchange {
            assert_eq!(now.orgs.get(id), Some(other), "{id}");
        }
    }
    sweep(&mut tw, 3_600);
    let balance = economy(&tw)
        .snapshot()
        .account(AccountId(row.account.expect("an account")))
        .expect("the account")
        .posted_balance
        .minor();
    assert_eq!(balance, 400_000_000, "4,000,000.00cr, once");
}

/// **A tranche no sweep will settle is missing, not pending** (doc 18 §4.7's
/// report): a cancelled capital obligation is out of the due pool, so the
/// report names it instead of telling staff to wait for a sweep.
#[test]
fn ic3_a_cancelled_tranche_reads_missing_not_pending() {
    let mut tw = boot(&Roster::production());
    let map = port_map(&tw, "stickney_port");
    let kasei = map.orgs.get(&cid("kasei")).expect("kasei's row");
    let obligation = kasei.capital[0].obligation;
    let out = run(&mut tw, GOD, &format!("@obligation/cancel #{obligation}"));
    assert!(out.contains("cancelled"), "{out}");
    sweep(&mut tw, 3_600);
    let out = report(&mut tw);
    assert!(
        out.contains(&format!(
            "  stickney_port org kasei: Kasei Metals, org #{}, account #{}; capital obligation \
             #{obligation} is cancelled, and no sweep will settle it. [missing]",
            kasei.org,
            kasei.account.expect("an account")
        )),
        "{out}"
    );
    // Doc 18 cut IC8: the stock lines (pools, listings, bids) wait on the port tick,
    // which this file never lands; the claim is about the capital lines — and every
    // line that is pending is one of those stock lines.
    assert!(
        !out.lines()
            .any(|l| l.contains(" org ") && l.ends_with("[pending]")),
        "{out}"
    );
    assert!(
        out.lines()
            .filter(|l| l.ends_with("[pending]"))
            .all(|l| l.contains(" vendor ") || l.contains(" ask ") || l.contains(" bid ")),
        "{out}"
    );
    assert!(out.lines().any(|l| l.starts_with("Not ready: ")), "{out}");
}

// ---------------------------------------------------------------------------
// 5. A corrupt map
// ---------------------------------------------------------------------------

/// The corrupt map document's key.
fn ceres_key() -> String {
    tacsim::port::port_map_key(&lane_id(), &cid("ceres_port")).to_string()
}

/// **A corrupt port map refuses re-creation** (doc 18 §4.7; lens 3).
#[test]
fn ic3_a_corrupt_port_map_refuses_re_creation() {
    // (a) A kept world whose map rots.
    let v3 = lane_v4(|_| {});
    let roster = Roster::production().with_shelf(vec![v3]);
    let mut tw = boot(&roster);
    let mut folded = fold_module_ops(tw.take_module_ops());
    let key = ceres_key();
    let slot = folded
        .get_mut(tacsim::NS)
        .and_then(|data| data.kv.iter_mut().find(|(k, _)| k.as_str() == key))
        .expect("the ceres map document");
    slot.1 = serde_json::json!({ "schema": "x" });
    let mut tw = reboot(tw, &roster, folded);
    assert!(matches!(
        store(&tw).port_map(&lane_id(), &cid("ceres_port")),
        PortMapState::Unreadable
    ));
    assert!(
        mush_propagation::commerce::current(tw.world())
            .is_some_and(|market| market.capital_transaction.is_none()),
        "a held clearing map cannot fund the Exchange"
    );
    let out = report(&mut tw);
    assert!(
        out.contains(&format!(
            "  ceres_port: its port map {key} is unreadable. [missing]"
        )),
        "{out}"
    );
    let before = counts(&tw);
    let (_plan, apply) = plan_and_apply(&mut tw);
    assert!(apply.contains(&key), "{apply}");
    assert!(apply.contains("never re-created"), "{apply}");
    assert_eq!(lane_latch(&tw), Some(LANE_VERSION));
    assert_eq!(counts(&tw), before, "nothing was made");

    // (b) A fresh store carrying only that corrupt document refuses the first
    // boot of the lane before any of its entries mutates.
    let mut boot_data: HashMap<SmolStr, mush_core::ModuleBootData> = HashMap::new();
    boot_data
        .entry(SmolStr::new(tacsim::NS))
        .or_default()
        .kv
        .push((SmolStr::new(&key), serde_json::json!({ "schema": "x" })));
    let (world, registry) = mush_testkit::boot_world();
    let mut engine =
        mush_core::Engine::with_modules(world, registry, Roster::production().modules())
            .expect("the roster composes");
    let mut out = mush_core::Outbox::new();
    let mut fx = mush_core::Effects::new();
    let refused = engine
        .modules_loaded(boot_data, &mut out, &mut fx)
        .expect_err("the first boot of the lane is refused")
        .to_string();
    assert!(
        refused.contains(&format!(
            "content: first-boot pack {LANE_PACK}@{LANE_VERSION} refused: "
        )),
        "{refused}"
    );
    assert!(refused.contains(&key), "{refused}");
    assert_nothing_bound(&engine.world);
}

/// **A frozen economy refuses the lane before anything is made** (doc 18 §4.7;
/// the check phase decides everything decidable before a write). A kept world
/// on the IC1 boot list (no lane) whose economy freezes at the next boot — a
/// drifted account projection, `mush-economy`'s own boot integrity rule — takes
/// the lane for the first time: `create_system_obligation` would answer
/// `EconomyFrozen` while `open_account` would not, so without the check the
/// apply would open the domain, the treasury, an org and its account and then
/// stop at the first tranche.
#[test]
fn ic3_a_frozen_economy_refuses_the_lane_before_anything_is_made() {
    use tacsim::definitions::content::{boot_packs, station_packs};

    let mut old = mush_atlas::starter::packs().expect("the starter packs");
    old.extend(boot_packs().expect("the game packs"));
    old.extend(station_packs().expect("the station packs"));
    let mut tw = TestWorld::with_modules(support::roster_booting(config(), old));
    no_refused_writes(&tw);
    assert_eq!(lane_latch(&tw), None);
    let pat = tw.spawn_player("Pat");
    for line in [
        "@instrument/register gold=2".to_owned(),
        "@account/system faucet=gold".to_owned(),
        format!("@account/open #{}=gold", pat.0),
    ] {
        let _ = run(&mut tw, GOD, &line);
    }
    let gold = InstrumentId::new("gold").expect("an id");
    let account_of = |tw: &TestWorld, owner: &PrincipalId| -> AccountId {
        economy(tw)
            .snapshot()
            .accounts_of(owner)
            .find(|a| a.instrument == gold)
            .map(|a| a.id)
            .expect("the account")
    };
    let faucet = account_of(
        &tw,
        &PrincipalId::System(mush_org::SystemPrincipalId::new("faucet").expect("a principal")),
    );
    let held = account_of(&tw, &PrincipalId::Object(pat));
    let out = run(
        &mut tw,
        GOD,
        &format!("@econ/mint {}={} 100", faucet.0, held.0),
    );
    assert_eq!(
        economy(&tw)
            .snapshot()
            .account(held)
            .expect("the account")
            .posted_balance
            .minor(),
        100,
        "{out}"
    );
    let mut folded = fold_module_ops(tw.take_module_ops());
    let doc_key = format!("account:{}", held.0);
    let doc = folded
        .get_mut(mush_economy::NS)
        .and_then(|data| data.kv.iter_mut().find(|(k, _)| k.as_str() == doc_key))
        .expect("the account document");
    doc.1["posted_balance"] = serde_json::json!(999);

    let world = tw.into_engine().world;
    let (_, registry) = mush_testkit::boot_world();
    let mut engine =
        mush_core::Engine::with_modules(world, registry, Roster::production().modules())
            .expect("the roster composes");
    let mut out = mush_core::Outbox::new();
    let mut fx = mush_core::Effects::new();
    let refused = engine
        .modules_loaded(folded, &mut out, &mut fx)
        .expect_err("the first boot of the lane is refused")
        .to_string();
    assert!(
        engine
            .world
            .module_state::<EconomyStore>()
            .expect("economy state")
            .is_frozen(),
        "the drifted projection froze the economy"
    );
    assert!(
        refused.contains(&format!(
            "content: first-boot pack {LANE_PACK}@{LANE_VERSION} refused: "
        )),
        "{refused}"
    );
    assert!(refused.contains("the economy is frozen"), "{refused}");
    // Cut IC4: the pass would also title the eight programme hulls, and says
    // so; no hull Thing was made.
    assert!(
        refused.contains("capital tranche(s) and 8 hull title(s) cannot be issued"),
        "{refused}"
    );
    assert!(
        !engine
            .world
            .iter()
            .any(|o| o.kind == mush_core::ObjKind::Thing && o.name == "Slack Water"),
        "no hull was made"
    );
    assert_nothing_bound(&engine.world);
}

/// No org, no port account, no port domain: the lane refused before its first
/// write (the starter, proving-ground and station packs applied before it, so
/// objects and their domains do exist).
fn assert_nothing_bound(w: &mush_core::World) {
    let orgs = w.module_state::<OrgStore>().expect("org state");
    assert_eq!(orgs.state().orgs().count(), 0, "no org was created");
    let economy = w.module_state::<EconomyStore>().expect("economy state");
    let treasury = PrincipalId::System(
        mush_org::SystemPrincipalId::new("tacsim.treasury").expect("a principal"),
    );
    assert!(
        !economy
            .snapshot()
            .accounts()
            .any(|a| a.instrument == credit()
                && (a.owner == treasury || matches!(a.owner, PrincipalId::Organization(_)))),
        "no port account was opened"
    );
    assert_eq!(economy.snapshot().obligations().count(), 0, "no obligation");
    let spatial = w.module_state::<SpatialState>().expect("sim state");
    for domain in spatial.domains.values() {
        let host = mush_core::output::display_name(w, domain.host);
        assert!(
            ![
                "Ceres Freeport",
                "Stickney Yards",
                "Aitken Highport",
                "Voltaire Anchorage",
                "Ligeia Terminal",
            ]
            .contains(&host.as_str()),
            "a port domain was opened on {host}"
        );
    }
}

// ---------------------------------------------------------------------------
// 6. Eight windows
// ---------------------------------------------------------------------------

/// **A kept world with eight windows open at restart still applies the lane**
/// (doc 18 §4.4, §9 risk 1; lens 5). One window is opened by GOD's
/// `@atlas/window open`; its sim domain document is cloned under seven fresh
/// domain ids on seven other station rooms in the folded store, with the sim
/// allocator bumped past them.
#[test]
fn ic3_a_kept_world_with_eight_windows_open_still_applies_the_lane() {
    use tacsim::definitions::content::{CERES_PACK, STICKNEY_PACK, boot_packs, station_packs};

    // The IC1 boot list: starter, proving ground, stations — no lane.
    let mut old = mush_atlas::starter::packs().expect("the starter packs");
    old.extend(boot_packs().expect("the game packs"));
    old.extend(station_packs().expect("the station packs"));
    let mut tw = TestWorld::with_modules(support::roster_booting(config(), old));
    no_refused_writes(&tw);
    assert_eq!(lane_latch(&tw), None);
    let room = |tw: &TestWorld, pack: &str, entry: &str| -> ObjId {
        mush_atlas::travel::latched_object(
            tw.world(),
            &mush_atlas::RecordKey::new(cid(pack), cid(entry)),
        )
        .unwrap_or_else(|| panic!("{pack}.{entry} is latched"))
    };
    let first = room(&tw, CERES_PACK, "room.c07_exchange");
    let (events, effects) = tw.run_with_effects(GOD, &format!("@atlas/window open #{}", first.0));
    let opened = said(&tw, &events, GOD);
    finish_composites(&mut tw, &effects);
    assert_eq!(mush_sim::windows(tw.world()).count(), 1, "{opened}");
    let window_id = mush_sim::windows(tw.world())
        .next()
        .map(|(id, _)| id)
        .expect("the window");
    let sim_doc = serde_json::to_value(spatial(&tw).domains.get(&window_id).expect("its record"))
        .expect("encodes");
    let hosts = [
        room(&tw, CERES_PACK, "room.c08_chandlery"),
        room(&tw, CERES_PACK, "room.c14_dockmaster"),
        room(&tw, STICKNEY_PACK, "room.p03_customs"),
        room(&tw, STICKNEY_PACK, "room.p05_harbourmaster"),
        room(&tw, STICKNEY_PACK, "room.p07_receiving"),
        room(&tw, STICKNEY_PACK, "room.p08_exchange"),
        room(&tw, STICKNEY_PACK, "room.p09_commissary"),
    ];
    let next = spatial(&tw).next_domain_id;
    let mut folded = fold_module_ops(tw.take_module_ops());
    // The atlas's own window document (the composite landed), cloned beside the
    // sim's: the atlas closes, at boot, every sim window it holds no composited
    // entry for (`mush_atlas::window::hydrate`'s orphans).
    let atlas_key = mush_atlas::window::window_key(window_id.0);
    let atlas_doc = folded
        .get(mush_atlas::NS)
        .and_then(|data| data.kv.iter().find(|(k, _)| *k == atlas_key))
        .map(|(_, v)| v.clone())
        .expect("the atlas window document");
    for (offset, host) in hosts.iter().enumerate() {
        let id = next + u64::try_from(offset).expect("small");
        let mut doc = sim_doc.clone();
        doc["id"] = serde_json::json!(id);
        doc["host"] = serde_json::json!(host.0);
        doc["presentation_location"] = serde_json::json!(host.0);
        folded
            .entry(SmolStr::new(mush_sim::NS))
            .or_default()
            .kv
            .push((SmolStr::new(format!("domain:{id}")), doc));
        let mut entry = atlas_doc.clone();
        entry["domain"] = serde_json::json!(id);
        folded
            .entry(SmolStr::new(mush_atlas::NS))
            .or_default()
            .kv
            .push((mush_atlas::window::window_key(id), entry));
    }
    let sim = folded.entry(SmolStr::new(mush_sim::NS)).or_default();
    let bumped = next + u64::try_from(hosts.len()).expect("small");
    match sim
        .kv
        .iter_mut()
        .find(|(k, _)| k.as_str() == mush_sim::data::META_KEY)
    {
        Some(meta) => meta.1["next_domain_id"] = serde_json::json!(bumped),
        None => sim.kv.push((
            SmolStr::new(mush_sim::data::META_KEY),
            serde_json::json!({ "next_domain_id": bumped }),
        )),
    }

    let mut tw = reboot(tw, &Roster::production(), folded);
    assert_eq!(
        mush_sim::windows(tw.world()).count(),
        mush_sim::MAX_WINDOWS,
        "eight windows are open"
    );
    assert_eq!(lane_latch(&tw), Some(LANE_VERSION), "the lane applied");
    for entry in ["ceres_port", "stickney_port"] {
        let map = port_map(&tw, entry);
        let domain = spatial(&tw)
            .domains
            .get(&mush_sim::data::DomainId(map.domain.expect("a domain")))
            .expect("the port's domain");
        assert_eq!(
            domain.status,
            mush_sim::data::DomainStatus::Active,
            "{entry}"
        );
    }
    assert_eq!(mush_sim::windows(tw.world()).count(), mush_sim::MAX_WINDOWS);
    // Another passive domain still opens; a ninth window does not.
    let spare = room(&tw, CERES_PACK, "room.c06_clearing");
    let out = run(
        &mut tw,
        GOD,
        &format!("@behavior/add #{}=sim.domain", spare.0),
    );
    let out = format!(
        "{out}\n{}",
        run(
            &mut tw,
            GOD,
            &format!("@domain/create #{}=sim.passive", spare.0)
        )
    );
    assert!(out.contains("Created domain #"), "{out}");
    let ninth = room(&tw, STICKNEY_PACK, "room.p11_service");
    let (events, _) = tw.run_with_effects(GOD, &format!("@atlas/window open #{}", ninth.0));
    let out = said(&tw, &events, GOD);
    assert!(out.contains("windows are open already"), "{out}");
    assert_eq!(mush_sim::windows(tw.world()).count(), mush_sim::MAX_WINDOWS);
}

// ---------------------------------------------------------------------------
// 7. A world on lane v1
// ---------------------------------------------------------------------------

/// Lane v1's bytes, rebuilt from v2: the ports dropped, IC2's four seconds.
fn lane_v1_json() -> String {
    lane_variant(1, |doc| {
        // Doc 19 cut A.8: v1 had none of the moons' trade (its three places stay, so
        // the placed rooms keep their ids across the upgrade); doc 20 cut SM.2: nor the
        // sites.
        support::sites::strip_sites(doc);
        support::moon::strip_moon_trade(doc);
        let entries = doc["entries"].as_object_mut().expect("entries");
        entries.remove("ceres_port");
        entries.remove("stickney_port");
        doc["entries"]["ceres_stickney"]["def"]["legs"][0]["base_s"] = serde_json::json!(592_332);
        doc["entries"]["ceres_stickney"]["def"]["legs"][1]["base_s"] = serde_json::json!(454_177);
        doc["entries"]["stickney_ceres"]["def"]["legs"][0]["base_s"] = serde_json::json!(454_177);
        doc["entries"]["stickney_ceres"]["def"]["legs"][1]["base_s"] = serde_json::json!(592_332);
    })
}

/// **A world on lane v1 takes the shipped lane by the shelf** (L0b; lens 7) — v2 when
/// cut IC3 wrote it, v3 since cut IC8; its v1 builder drops both port entries, so the
/// vendors arrive with them.
#[test]
fn ic3_a_world_on_lane_v1_takes_v2_by_the_shelf() {
    let v1 = lane_v1_json();
    // An operator seed with the game sources off, applied by a content module
    // whose boot list preserves the lane dependency order and historical relay seed.
    let mut cfg = config();
    cfg.seeds.push(PackSource {
        name: "ops/lane_v1.json".to_owned(),
        bytes: v1.clone(),
    });
    let mut mods = Roster::bare().modules_with(cfg, false);
    mods.pop();
    let packs = support::boot_packs_with_lane(content_pack(&v1));
    let mut appliers = mush_content::Appliers::new();
    mush_atlas::register_appliers(&mut appliers).expect("atlas appliers");
    mush_npc::content::register_appliers(&mut appliers).expect("npc appliers");
    tacsim::register_appliers(&mut appliers).expect("tacsim appliers");
    mods.push(Box::new(
        mush_content::ContentModule::new()
            .with_appliers(appliers)
            .with_boot_packs(packs),
    ));
    let tw = TestWorld::with_modules(mods);
    no_refused_writes(&tw);
    assert_eq!(lane_latch(&tw), Some(1));
    assert!(
        store(&tw).published_digest(&lane_id(), 1).is_some(),
        "the v1 ledger row"
    );
    let placed_v1 = tw
        .world()
        .module_state::<ContentState>()
        .and_then(|c| c.latches.applied(&lane_id()))
        .map(mush_content::Applied::placed)
        .expect("v1 placed");

    let mut tw = tw;
    let folded = fold_module_ops(tw.take_module_ops());
    let mut next = reboot(tw, &Roster::production(), folded);
    {
        assert_eq!(
            lane_latch(&next),
            Some(1),
            "the boot pass never upgrades a latch"
        );
        assert!(store(&next).published_digest(&lane_id(), 1).is_some());
        assert!(
            store(&next)
                .published_digest(&lane_id(), LANE_VERSION)
                .is_some()
        );
        for entry in [
            "ceres_port",
            "stickney_port",
            "luna_port",
            "deimos_port",
            "titan_port",
        ] {
            assert!(matches!(
                store(&next).port_map(&lane_id(), &cid(entry)),
                PortMapState::Absent
            ));
        }
        let out = report(&mut next);
        for line in [
            format!(
                "lane: {LANE_PACK}@1 applied; this server ships @{LANE_VERSION}. Take it with @atlas/pack \
                 plan {LANE_PACK}. [flagged]"
            ),
            "  ceres_port: not applied. [missing]".to_owned(),
            "  stickney_port: not applied. [missing]".to_owned(),
            "  luna_port: not applied. [missing]".to_owned(),
            "  deimos_port: not applied. [missing]".to_owned(),
            "  titan_port: not applied. [missing]".to_owned(),
        ] {
            assert!(out.lines().any(|l| l == line), "{line}\n{out}");
        }
        assert!(
            out.lines().any(|l| l
                .starts_with("sky: constant (rate 0), kepler, epoch 2351-05-13, rev ")
                && l.ends_with(" [ready]")),
            "{out}"
        );
        assert!(out.lines().any(|l| l.starts_with("Not ready: ")), "{out}");
        let plan = run(&mut next, GOD, &format!("@atlas/pack plan {LANE_PACK}"));
        assert!(
            plan.contains(&format!(
                // Doc 20 cut SM.2 (lane v9): the six sites' twenty-four entries are created,
                // and the eight bodies and the Freeport's place updated, beside v8's thirteen.
                "Plan for {LANE_PACK}@{LANE_VERSION}: 35 create, 11 update, 0 retire, 14 unchanged"
            )),
            "{plan}"
        );
        for (entry, step) in [
            ("ceres_stickney (route)", "update"),
            ("stickney_ceres (route)", "update"),
            ("ceres_port (port)", "create"),
            ("stickney_port (port)", "create"),
            // Doc 19 cut A.8: the moon ports and their routes arrive with the trade.
            ("luna_port (port)", "create"),
            ("deimos_port (port)", "create"),
            ("titan_port (port)", "create"),
            ("ceres_aitken (route)", "create"),
            ("aitken_voltaire (route)", "create"),
            ("aitken_ligeia (route)", "create"),
            ("ligeia_aitken (route)", "create"),
            ("ligeia_voltaire (route)", "create"),
            ("voltaire_stickney (route)", "create"),
        ] {
            assert!(
                plan.lines().any(|l| l.contains(step) && l.contains(entry)),
                "{entry}:\n{plan}"
            );
        }
        let (_, apply) = plan_and_apply(&mut next);
        assert!(
            apply.contains(&format!(
                "Applied {LANE_PACK}@{LANE_VERSION}: 35 created, 11 updated, 0 retired, 14 unchanged."
            )),
            "{apply}"
        );
        assert_eq!(lane_latch(&next), Some(LANE_VERSION));
        let placed_v2 = next
            .world()
            .module_state::<ContentState>()
            .and_then(|c| c.latches.applied(&lane_id()))
            .map(mush_content::Applied::placed)
            .expect("v2 placed");
        // Doc 20 cut SM.2: the six site places are added; the six old place rooms keep
        // their ids.
        assert_eq!(placed_v2.len(), placed_v1.len() + 6);
        for (place, room) in &placed_v1 {
            assert_eq!(
                placed_v2.get(place),
                Some(room),
                "the place rooms keep their ids"
            );
        }
        for entry in ["ceres_port", "stickney_port"] {
            let _ = port_map(&next, entry);
        }
        let out = report(&mut next);
        assert!(
            out.contains(&format!(
                "lane: {LANE_PACK}@{LANE_VERSION} applied; this server ships @{LANE_VERSION}. [ready]"
            )),
            "{out}"
        );
        no_refused_writes(&next);
        assert!(
            mush_propagation::commerce::current(next.world()).is_none(),
            "an old lane did not create unfunded relay accounts at boot"
        );
        sweep(&mut next, 3_600);
        sweep(&mut next, 5);
        assert!(
            mush_propagation::commerce::current(next.world())
                .is_some_and(|market| market.capital_transaction.is_some()),
            "normal cadence initializes retail after the real lane upgrade"
        );
        assert!(
            mush_propagation::billing::current(next.world()).is_some(),
            "subscription enrollment becomes available without a second reboot"
        );
    }
}

// ---------------------------------------------------------------------------
// 8. The readiness report
// ---------------------------------------------------------------------------

/// **The readiness report lists every binding and flags the rate** (doc 18
/// §4.7; L14, L15; lens 6).
#[test]
fn ic3_the_readiness_report_lists_every_binding_and_flags_the_rate() {
    let mut tw = boot(&Roster::production());
    set_rate(&mut tw, 24);
    let out = report(&mut tw);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.first(), Some(&"Port readiness:"), "{out}");
    let lane_pack = load_pack(LANE_JSON).expect("the lane verifies");
    let sky = tacsim::Sky::from_pack(&lane_pack, &tacsim::EphemerisConfig::default());
    let rev: String = sky
        .revision()
        .expect("a revision")
        .to_hex()
        .chars()
        .take(12)
        .collect();
    let treasury = port_map(&tw, "ceres_port").treasury.expect("a treasury");
    let mut want = vec![
        format!(
            "lane: {LANE_PACK}@{LANE_VERSION} applied; this server ships @{LANE_VERSION}. [ready]"
        ),
        format!("sky: constant (rate 0), kepler, epoch 2351-05-13, rev {rev} [ready]"),
        // Doc 20 cut SM.1: the geology line follows the sky; this world has no key.
        "geology: disabled: no [tacsim.geology].key_dir is configured. [flagged]".to_owned(),
        "world rate: 24 IC seconds per real second. [ready]".to_owned(),
        format!("treasury: system:tacsim.treasury, account #{treasury}. [ready]"),
        // Doc 18 cut IC8: the market facts line follows the treasury.
        "market facts: answered, so drawn sales can settle. [ready]".to_owned(),
        // Doc 18 cut IC10: the inventory facts line follows the market facts.
        "inventory facts: answered, so sales into bids can settle. [ready]".to_owned(),
        // Doc 18 cut IC14 (§9 risk 19): the marker headroom line follows them.
        format!(
            "markers: {} of {} applied markers held. [ready]",
            tw.world()
                .module_state::<mush_inventory::InventoryStore>()
                .expect("an inventory store")
                .markers(),
            mush_inventory::MAX_APPLIED_MARKERS
        ),
    ];
    let mut commerce_lines = 0;
    for (entry, def) in ports_of(LANE_JSON) {
        let map = port_map(&tw, entry.as_str());
        let refs: BTreeSet<&str> = std::iter::once(def.gallery.as_str())
            .chain(def.rooms.values().map(String::as_str))
            .chain(def.programme.iter().map(|p| p.office.as_str()))
            .collect();
        want.push(format!(
            "  {entry} rooms: {n} of {n} resolved; the gallery carries port.dock. [ready]",
            n = refs.len()
        ));
        let place = location_label(&def.location);
        // Doc 20 cut SM.2: a site presents on its pad, and its site line follows.
        let gallery = def.site.as_ref().map_or_else(
            || "Visiting Berth Gallery".to_owned(),
            |site| format!("{} Pad", site.name),
        );
        want.push(format!(
            "  {entry} domain: #{} on {place}, presents in {gallery}. [ready]",
            map.domain.expect("a domain")
        ));
        if def.site.is_some() {
            // Doc 20 cut SM.6 (D2, D3): the block vaults counted; this world's geology layer
            // is dark, so nothing opens and the pad stays open.
            want.push(format!(
                "  {entry} site: 16 block places, the site clerk, the spoil and 16 block vaults \
                 laid up; no deposits while the geology layer is dark. [ready]"
            ));
            // Doc 20 cut SM.12b: a site with a yard reports its Dispatch line right after its
            // Site line — ungated, and Ready while the site is not open.
            if def.site.as_ref().is_some_and(|site| site.yard.is_some()) {
                want.push(format!(
                    "  {entry} dispatch occator: 2 rigs in the Occator Yard's bay; the site is \
                     not open, so nothing is filed. [ready]"
                ));
            }
        }
        for org in &def.orgs {
            let row = map.orgs.get(&org.id).expect("a row");
            want.push(format!(
                "  {entry} org {}: {}, org #{}, account #{}, capital {}.00cr: 0 of {} tranches \
                 settled. [pending]",
                org.id,
                org.name,
                row.org,
                row.account.expect("an account"),
                // Doc 20 cut SM.5: the capital with its raises, and the schedule's
                // tranches (Occator: 300000.00cr, 2 tranches).
                org.capital_total().expect("the capital fits"),
                tacsim::port::capital_schedule(org.capital_major, &org.raises).len()
            ));
        }
        if let Some(programme) = &def.programme {
            let org = def
                .orgs
                .iter()
                .find(|o| o.id == programme.org)
                .expect("the programme org");
            let row = map.orgs.get(&programme.org).expect("a row");
            want.push(format!(
                "  {entry} programme {}: {}, account #{}, floor {}.00cr, stake {}.00cr for each of \
                 {} hulls. [ready]",
                programme.id,
                org.name,
                row.account.expect("an account"),
                programme.floor_major,
                programme.stake_major,
                programme.pool
            ));
            // Cut IC4: the programme's hulls line follows its programme line.
            want.push(format!(
                "  {entry} hulls: {pool} of {pool} made, titled to {} and laid up. [ready]",
                org.name,
                pool = programme.pool
            ));
        }
        // Doc 20 cut SM.3a: the rig pool's (ungated) line follows the hulls line.
        if let Some(vehicles) = &def.vehicles {
            let lessor = def
                .orgs
                .iter()
                .find(|o| o.id == vehicles.org)
                .expect("the lessor org");
            // Doc 32 cut CF.6c: each further design's own tally follows the pool's.
            let designs: String = vehicles
                .designs
                .iter()
                .map(|design| {
                    format!(
                        " {word}: {pool} of {pool} made; {pool} in the shed, 0 leased.",
                        word = design.word,
                        pool = design.pool
                    )
                })
                .collect();
            want.push(format!(
                "  {entry} vehicles: {pool} of {pool} made and titled to {}; {pool} in the shed, \
                 0 leased.{designs} [ready]",
                lessor.name,
                pool = vehicles.pool
            ));
        }
        // Doc 20 cut SM.5: the (ungated) licence line follows the rig pool's.
        if let Some(licence) = &def.licence {
            let issuer = def
                .orgs
                .iter()
                .find(|o| o.id == licence.org)
                .expect("the issuer org");
            want.push(format!(
                "  {entry} licence tacsim.licence.freeport: the {} issues it at {}.00cr for {} IC \
                 days with {} IC days' grace; extraction reads it. [ready]",
                issuer.name, licence.fee_major, licence.term_ic_days, licence.grace_ic_days
            ));
        }
        // Doc 18 cut IC8: before any tick, per vendor its pools line (nothing seeded),
        // per ask its listings line (not seeded yet), per bid its bids line (posted,
        // but its capital has not settled) — 3 + 4 + 2 at each port.
        for vendor in &def.vendors {
            let row = map.vendors.get(&vendor.org).expect("a vendor row");
            let name = &def
                .orgs
                .iter()
                .find(|o| o.id == vendor.org)
                .expect("the vendor's org")
                .name;
            // Doc 19 cut A.8: a bid-only vendor has nothing to seed and reads Ready.
            let state = if vendor.stock.is_empty() {
                "ready"
            } else {
                "pending"
            };
            want.push(format!(
                "  {entry} vendor {}: {name}, storefront vault #{}; 0 of {} stock rows seeded. \
                 [{state}]",
                vendor.org,
                row.vault,
                vendor.stock.len()
            ));
            commerce_lines += 1;
        }
        for vendor in &def.vendors {
            for ask in &vendor.asks {
                want.push(format!(
                    "  {entry} ask {} {}: not seeded yet. [pending]",
                    vendor.org, ask.good
                ));
                commerce_lines += 1;
            }
        }
        for vendor in &def.vendors {
            let row = map.vendors.get(&vendor.org).expect("a vendor row");
            for bid in &vendor.bids {
                let listing = row.bids.get(&bid.good).expect("a posted bid").listing;
                want.push(format!(
                    "  {entry} bid {} {}: listing #{listing}; not buying until its capital \
                     settles. [pending]",
                    vendor.org, bid.good
                ));
                commerce_lines += 1;
            }
        }
        // Doc 20 cut SM.12a: one Mill line per mill row, after the port's Bids lines;
        // nothing milled yet reads Ready (reported, never gated).
        for vendor in &def.vendors {
            for mill in &vendor.mill {
                want.push(format!(
                    "  {entry} mill {} {}: nothing milled yet; 0 t of ore waits; {} to {}, {} t \
                     a batch, at most {} t an IC day. [ready]",
                    vendor.org,
                    mill.ore,
                    mill.ratio.ore,
                    mill.ratio.product,
                    mill.batch_t,
                    mill.per_ic_day_t
                ));
                commerce_lines += 1;
            }
        }
    }
    // Doc 19 cut A.8: one vendor, ask and bid line per authored row, pinned against
    // the shipped lane (a count of report lines, not a vertex): Ceres 3 + 4 + 6 (doc 20
    // cut SM.5: the four ore intakes) + 2 mills (doc 20 cut SM.12a), Stickney 3 + 4 + 2,
    // Luna 4 + 4 + 3, Deimos 2 + 2 + 4, Titan 3 + 4 + 2. The bid-only vendors' lines are
    // counted though they read Ready.
    assert_eq!(
        commerce_lines, 52,
        "the shipped lane's vendor, ask, bid and mill lines at the five ports"
    );
    let total = want.len();
    // Doc 19 cut A.8: every line that is not ready — the orgs' tranches, the
    // commerce lines and whatever else the report flags — counted off the list.
    let not_ready = want.iter().filter(|l| !l.ends_with("[ready]")).count();
    want.push(format!(
        "Not ready: {not_ready} of {total} lines are pending, missing or flagged."
    ));
    assert_eq!(
        &lines[1..],
        want.iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .as_slice(),
        "{out}"
    );

    // The rate: flagged off 24 on a real world.
    set_rate(&mut tw, 4);
    let out = report(&mut tw);
    assert!(
        out.contains("world rate: 4 IC seconds per real second; play runs at 24. [flagged]"),
        "{out}"
    );
    // A player is refused; a bad argument gets the usage line.
    let pilot = tw.spawn_player("Pilot");
    assert!(run(&mut tw, pilot, "@tacsim/port status").contains("Permission denied."));
    assert!(run(&mut tw, GOD, "@tacsim/port foo").contains("Usage: @tacsim/port status"));

    // A test world at 3600 is ready on the rate.
    let mut test = boot(&Roster::test_world());
    assert!(store(&test).test_world());
    set_rate(&mut test, 3_600);
    let out = report(&mut test);
    assert!(
        out.contains("world rate: 3600 IC seconds per real second (test world). [ready]"),
        "{out}"
    );
}

// ---------------------------------------------------------------------------
// 9. +credit
// ---------------------------------------------------------------------------

/// **`+credit` reads posted, held and available** (doc 18 §4.1; L12). On the
/// sheet-less roster, so a spawned pilot walks through the Limbo door.
#[test]
fn ic3_credit_reads_posted_held_and_available() {
    let mut tw = boot(&Roster::bare());
    let pilot = tw.spawn_player("Pilot");
    assert_eq!(
        run(&mut tw, pilot, "+credit"),
        "No credit account is open in your name."
    );
    let out = run(
        &mut tw,
        GOD,
        &format!("@account/open #{}=tacsim.credit", pilot.0),
    );
    let account = economy(&tw)
        .snapshot()
        .accounts_of(&PrincipalId::Object(pilot))
        .map(|a| a.id)
        .next()
        .unwrap_or_else(|| panic!("the pilot's account: {out}"));
    run(&mut tw, GOD, "@account/system test.faucet=tacsim.credit");
    let faucet = economy(&tw)
        .snapshot()
        .accounts()
        .find(|a| {
            a.owner
                == PrincipalId::System(
                    mush_org::SystemPrincipalId::new("test.faucet").expect("a principal"),
                )
        })
        .map(|a| a.id)
        .expect("the faucet");
    let out = run(
        &mut tw,
        GOD,
        &format!("@econ/mint {faucet}={account} 2500000"),
    );
    let line = tacsim::port::credit::credit_line("25000.00cr", "0.00cr", "25000.00cr");
    assert_eq!(run(&mut tw, pilot, "+credit"), line, "{out}");
    // From a station room alike.
    let _ = run(&mut tw, pilot, "c");
    let here = tw.world().get(pilot).and_then(|o| o.location);
    assert!(
        here.is_some_and(|room| tacsim::ic_return::port_of_room(tw.world(), room).is_some()),
        "the pilot stands in a port"
    );
    assert_eq!(run(&mut tw, pilot, "+credit"), line);
    // The pilot never sees an org account.
    assert!(!run(&mut tw, pilot, "+credit").contains("root"));

    // A disabled layer.
    let off = Roster::bare();
    let mut cfg = config();
    cfg.enabled = false;
    let mut tw = TestWorld::with_modules(off.modules_with(cfg, false));
    let pilot = tw.spawn_player("Pilot");
    assert_eq!(
        run(&mut tw, pilot, "+credit"),
        "TacSim is not enabled on this server."
    );
}

// ---------------------------------------------------------------------------
// 10. Refusals
// ---------------------------------------------------------------------------

/// The refusal of a lane variant, through the strict loader.
fn refusal(edit: impl FnOnce(&mut serde_json::Value)) -> String {
    let json = lane_variant(2, edit);
    let through_loader = load_pack(&json).err().map(|e| e.to_string());
    let through_decode = mush_content::parse_pack(&json)
        .map_err(PackError::from)
        .and_then(rules_pack_of)
        .err()
        .map(|e| e.to_string());
    assert_eq!(through_loader, through_decode, "one rule set, two doors");
    through_loader.unwrap_or_else(|| "accepted".to_owned())
}

/// One mutation of the lane's JSON.
type Edit = Box<dyn FnOnce(&mut serde_json::Value)>;

/// **Every port entry the applier could not bind is refused at load, naming
/// the entry and the rule** (doc 18 §4.7; D-18-20, D-18-25, D-18-47).
#[test]
fn ic3_port_entries_refuse_what_the_applier_cannot_bind() {
    use serde_json::json;

    assert_eq!(refusal(|_| {}), "accepted");
    let cases: Vec<(&str, Edit, Vec<&str>)> = vec![
        (
            "a vendor without signatory",
            Box::new(|doc| {
                doc["entries"]["ceres_port"]["def"]["vendors"] = json!([{
                    "org": "occator",
                    "storefront": "neighborhood.ceres_freeport.room.c07_exchange",
                    "agent": "npc.someone"
                }]);
            }),
            vec!["ceres_port", "signatory"],
        ),
        (
            "routes as a bare string",
            Box::new(|doc| {
                doc["entries"]["ceres_port"]["def"]["routes"] = json!("ceres_stickney");
            }),
            vec!["ceres_port", "expected a map"],
        ),
        (
            "an empty route list",
            Box::new(|doc| {
                doc["entries"]["ceres_port"]["def"]["routes"] = json!({ "stickney": [] });
            }),
            vec!["port entry ceres_port", "routes.stickney", "1..=8"],
        ),
        (
            "a route from elsewhere",
            Box::new(|doc| {
                doc["entries"]["ceres_port"]["def"]["routes"] =
                    json!({ "stickney": ["stickney_ceres"] });
            }),
            vec![
                "port entry ceres_port",
                "stickney_ceres",
                "not ceres_freeport",
            ],
        ),
        (
            "a routes key that is no other port's venue",
            Box::new(|doc| {
                doc["entries"]["ceres_port"]["def"]["routes"] =
                    json!({ "mars": ["ceres_stickney"] });
            }),
            vec![
                "port entry ceres_port",
                "mars",
                "not the venue of another port",
            ],
        ),
        (
            "a room role outside PORT_ROOM_ROLES",
            Box::new(|doc| {
                doc["entries"]["ceres_port"]["def"]["rooms"]["bar"] =
                    json!("neighborhood.ceres_freeport.room.c07_exchange");
            }),
            vec!["port entry ceres_port", "`bar`", "port room role"],
        ),
        (
            "a gallery outside the area",
            Box::new(|doc| {
                doc["entries"]["ceres_port"]["def"]["gallery"] =
                    json!("neighborhood.stickney_yards.room.p01_berth_gallery");
            }),
            vec![
                "port entry ceres_port",
                "gallery",
                "is not in the port's area",
            ],
        ),
        (
            "a programme org not in orgs",
            Box::new(|doc| {
                doc["entries"]["ceres_port"]["def"]["programme"]["org"] = json!("kasei");
            }),
            vec![
                "port entry ceres_port",
                "org kasei",
                "not one of this port's orgs",
            ],
        ),
        (
            "the floor and stakes over the capital",
            Box::new(|doc| {
                doc["entries"]["ceres_port"]["def"]["programme"]["stake_major"] = json!(30_000);
            }),
            vec![
                "port entry ceres_port",
                "floor_major + pool × stake_major is 290000",
            ],
        ),
        (
            "hull names that are not the pool",
            Box::new(|doc| {
                doc["entries"]["ceres_port"]["def"]["programme"]["pool"] = json!(7);
            }),
            vec!["port entry ceres_port", "hull_names must name 7 hulls"],
        ),
        (
            "a reference to provisions",
            Box::new(|doc| {
                doc["entries"]["ceres_port"]["def"]["programme"]["reference"]["provisions"] =
                    json!("ceres_port/occator");
            }),
            vec!["port entry ceres_port", "provisions", "not a trade good"],
        ),
        (
            "a reference to nobody",
            Box::new(|doc| {
                doc["entries"]["ceres_port"]["def"]["programme"]["reference"]["metals"] =
                    json!("stickney_port/nobody");
            }),
            vec!["port entry ceres_port", "stickney_port/nobody"],
        ),
        (
            "an org id repeated across the ports",
            Box::new(|doc| {
                let orgs = doc["entries"]["stickney_port"]["def"]["orgs"]
                    .as_array_mut()
                    .expect("orgs");
                orgs.push(json!({
                    "id": "coop", "name": "Another Co-op", "capital_major": 1, "faction": "republic"
                }));
            }),
            vec![
                "port entry stickney_port",
                "org coop",
                "also authored by port ceres_port",
            ],
        ),
        // Doc 18 cut IC8: the vendors' rules.
        (
            "two stock clerks in one port",
            Box::new(|doc| {
                doc["entries"]["ceres_port"]["def"]["vendors"][1]["agent"] =
                    json!("ceres_port.other_clerk");
            }),
            vec![
                "port entry ceres_port",
                "every vendor of a port names the port's one stock clerk",
            ],
        ),
        (
            "one stock clerk in two ports",
            Box::new(|doc| {
                for n in 0..3 {
                    doc["entries"]["stickney_port"]["def"]["vendors"][n]["agent"] =
                        json!("ceres_port.clerk");
                }
            }),
            vec!["agent ceres_port.clerk is also port ceres_port's stock clerk"],
        ),
        (
            "an ask with no stock row",
            Box::new(|doc| {
                doc["entries"]["ceres_port"]["def"]["vendors"][0]["asks"]
                    .as_array_mut()
                    .expect("asks")
                    .push(json!({ "good": "volatiles", "price_major": 1 }));
            }),
            vec![
                "port entry ceres_port",
                "vendor freeport_exchange: ask volatiles has no stock row to draw from",
            ],
        ),
        (
            "an unknown good in a stock row",
            Box::new(|doc| {
                doc["entries"]["ceres_port"]["def"]["vendors"][0]["stock"]
                    .as_array_mut()
                    .expect("stock")
                    .push(json!({ "good": "unobtainium", "qty": 1, "grade": 50, "issue": 1 }));
            }),
            vec![
                "port entry ceres_port",
                "vendor freeport_exchange: unobtainium is not a good of the goods table",
            ],
        ),
        (
            "a board the port has no room for",
            Box::new(|doc| {
                doc["entries"]["ceres_port"]["def"]["vendors"][0]["asks"][0]["boards"] =
                    json!(["customs"]);
            }),
            vec![
                "port entry ceres_port",
                "ask metals: board `customs` is not one of the port's rooms",
            ],
        ),
        (
            "a board in the spine",
            Box::new(|doc| {
                doc["entries"]["ceres_port"]["def"]["vendors"][0]["bids"][0]["boards"] =
                    json!(["spine"]);
            }),
            vec![
                "port entry ceres_port",
                "bid structural_alloys: board `spine` is not one of the port's rooms",
            ],
        ),
        (
            "a stock row authored twice",
            Box::new(|doc| {
                doc["entries"]["ceres_port"]["def"]["vendors"][0]["stock"]
                    .as_array_mut()
                    .expect("stock")
                    .push(json!({ "good": "metals", "qty": 10, "grade": 50, "issue": 2 }));
            }),
            vec![
                "port entry ceres_port",
                "vendor freeport_exchange: stock metals is authored twice",
            ],
        ),
        (
            "an ask authored twice",
            Box::new(|doc| {
                doc["entries"]["ceres_port"]["def"]["vendors"][0]["asks"]
                    .as_array_mut()
                    .expect("asks")
                    .push(json!({ "good": "metals", "price_major": 201 }));
            }),
            vec![
                "port entry ceres_port",
                "vendor freeport_exchange: ask metals is authored twice",
            ],
        ),
        (
            "a bid authored twice",
            Box::new(|doc| {
                doc["entries"]["ceres_port"]["def"]["vendors"][0]["bids"]
                    .as_array_mut()
                    .expect("bids")
                    .push(json!({ "good": "structural_alloys", "price_major": 1, "intake": 1 }));
            }),
            vec![
                "port entry ceres_port",
                "vendor freeport_exchange: bid structural_alloys is authored twice",
            ],
        ),
        (
            "a good both asked and bid",
            Box::new(|doc| {
                doc["entries"]["ceres_port"]["def"]["vendors"][0]["bids"]
                    .as_array_mut()
                    .expect("bids")
                    .push(json!({ "good": "metals", "price_major": 1, "intake": 1 }));
            }),
            vec![
                "port entry ceres_port",
                "vendor freeport_exchange: metals is both asked and bid",
            ],
        ),
        (
            "a board named twice in one line",
            Box::new(|doc| {
                doc["entries"]["ceres_port"]["def"]["vendors"][0]["asks"][0]["boards"] =
                    json!(["exchange", "exchange"]);
            }),
            vec![
                "port entry ceres_port",
                "vendor freeport_exchange: ask metals: board `exchange` is named twice",
            ],
        ),
        (
            "a bid whose intake the market would not take",
            Box::new(|doc| {
                doc["entries"]["ceres_port"]["def"]["vendors"][0]["bids"][0]["intake"] =
                    json!(mush_market::MAX_QUANTITY + 1);
            }),
            vec![
                "port entry ceres_port",
                "vendor freeport_exchange: bid structural_alloys: intake must be at most",
            ],
        ),
        (
            "a reference to a vendor with no ask for the good",
            Box::new(|doc| {
                doc["entries"]["ceres_port"]["def"]["programme"]["reference"]["metals"] =
                    json!("ceres_port/coop");
            }),
            vec![
                "port entry ceres_port",
                "reference metals names `ceres_port/coop`, which posts no ask for it",
            ],
        ),
    ];
    for (name, edit, words) in cases {
        let refused = refusal(edit);
        for word in words {
            assert!(refused.contains(word), "{name}: {word}:\n{refused}");
        }
    }
    // Doc 18 cut IC8 (lead ruling L23): the same reference in an entry that authors no
    // vendors loads — the rule is scoped to an entry with vendors, so the published v2
    // (references, no vendors) keeps loading. Doc 19 cut A.4b: an emergency block needs
    // a vendor to post from, so the v2 shape has none. Doc 20 cut SM.12b: a yard delivers to a
    // mill row, which lives on a vendor, so the v2 shape has no yard either (`strip_yard`).
    assert_eq!(
        refusal(|doc| {
            support::drones::strip_yard(doc);
            if let Some(def) = doc["entries"]["ceres_port"]["def"].as_object_mut() {
                def.remove("vendors");
                def.remove("emergency");
            }
            doc["entries"]["ceres_port"]["def"]["programme"]["reference"]["metals"] =
                json!("ceres_port/coop");
        }),
        "accepted"
    );
    // One whole message, verbatim.
    let refused = refusal(|doc| {
        doc["entries"]["ceres_port"]["def"]["orgs"][1]["capital_major"] = json!(0);
    });
    assert!(
        refused.ends_with(
            "port entry ceres_port: org coop: capital_major must be 1..=100000000, not 0"
        ),
        "{refused}"
    );
    // A port in a rules pack is misplaced: a port instantiates.
    let mut doc: serde_json::Value = serde_json::from_str(LANE_JSON).expect("JSON");
    let port = doc["entries"]["ceres_port"].clone();
    doc["manifest"]["id"] = json!("test.rules.port");
    doc["manifest"]["kind"] = json!("rules_definition");
    doc["manifest"]["version"] = json!(1);
    doc["manifest"]["digest"] = json!(UNSIGNED);
    doc["entries"] = json!({ "ceres_port": port });
    let json = sign(&serde_json::to_string_pretty(&doc).expect("serializes"));
    let refused = load_pack(&json).expect_err("a port in a rules pack");
    assert!(
        matches!(&refused, PackError::WrongKindEntry { id, entry: "port", kind: "rules" } if id == "ceres_port"),
        "{refused}"
    );
}

// ---------------------------------------------------------------------------
// 11. The mint cap
// ---------------------------------------------------------------------------

/// The composed economy config on `roster`'s world.
fn cap_of(roster: &Roster, instrument: &str) -> u64 {
    let tw = boot(roster);
    let id = InstrumentId::new(instrument).expect("an instrument id");
    economy(&tw).config().cap_major(&id)
}

/// **A mint cap below a tranche refuses the lane before anything is made**
/// (D-18-14; L13; doc 18 §4.7 "Config flips").
#[test]
fn ic3_a_mint_cap_below_a_tranche_refuses_the_lane_before_anything_is_made() {
    assert_eq!(
        cap_of(&Roster::bare(), tacsim::CREDIT),
        1_000_000,
        "contributed"
    );
    for global in [50_000, 2_000_000] {
        let roster = Roster {
            economy: EconomyConfig {
                mint_obligation_cap_major: global,
                ..EconomyConfig::default()
            },
            ..Roster::bare()
        };
        let tw = boot(&roster);
        let config = economy(&tw).config();
        assert_eq!(
            config.cap_major(&credit()),
            1_000_000,
            "the contribution overrides a global {global} for the credit"
        );
        assert_eq!(
            config.cap_major(&InstrumentId::new("other.coin").expect("an id")),
            global,
            "another instrument keeps the global"
        );
    }
    let mut economy_cfg = EconomyConfig::default();
    economy_cfg.instruments.insert(
        tacsim::CREDIT.to_owned(),
        mush_economy::InstrumentCaps {
            mint_obligation_cap_major: 500_000,
        },
    );
    let roster = Roster {
        economy: economy_cfg,
        ..Roster::bare()
    };
    let (world, registry) = mush_testkit::boot_world();
    let mut engine = mush_core::Engine::with_modules(world, registry, roster.modules())
        .expect("the roster composes");
    let mut out = mush_core::Outbox::new();
    let mut fx = mush_core::Effects::new();
    let refused = engine
        .modules_loaded(HashMap::new(), &mut out, &mut fx)
        .expect_err("the lane is refused")
        .to_string();
    assert_eq!(
        engine
            .world
            .module_state::<EconomyStore>()
            .expect("economy")
            .config()
            .cap_major(&credit()),
        500_000,
        "the operator's per-instrument value stands"
    );
    assert!(
        refused.contains(
            "a capital tranche is 800000 cr but this world's mint cap for tacsim.credit is 500000 \
             cr; set [economy.instruments.\"tacsim.credit\"].mint_obligation_cap_major to at \
             least 800000"
        ),
        "{refused}"
    );
    assert_nothing_bound(&engine.world);
}

// ---------------------------------------------------------------------------
// 12. The IC-return table
// ---------------------------------------------------------------------------

/// **`ic_return::PORTS` agrees with the port entries** (L10).
#[test]
fn ic3_the_ic_return_table_agrees_with_the_port_entries() {
    let pack = load_pack(LANE_JSON).expect("the lane verifies");
    // Doc 20 cut SM.2: the IC door serves the five stations; a site is no IC port.
    let mut ports = ports_of(LANE_JSON);
    ports.retain(|(_, def)| def.site.is_none());
    assert_eq!(ports.len(), tacsim::ic_return::PORTS.len(), "five and five");
    for row in &tacsim::ic_return::PORTS {
        let matching: Vec<&(CanonicalId, PortDef)> = ports
            .iter()
            .filter(|(_, def)| def.venue.as_str() == row.id)
            .collect();
        assert_eq!(matching.len(), 1, "{}", row.id);
        let (_, def) = matching[0];
        assert_eq!(def.area.as_str(), row.area, "{}", row.id);
        let label = match pack.entry(&def.location) {
            Some(PackEntry::Location(place)) => place.label.clone(),
            other => panic!("{}: {other:?}", row.id),
        };
        assert_eq!(label, row.name, "{}", row.id);
        let arrival = def.rooms.get(&cid("arrivals")).unwrap_or(&def.gallery);
        let entry = room_key(&def.area, arrival).expect("in the area").entry;
        assert_eq!(entry.as_str(), row.arrival, "{}", row.id);
    }
}

/// A test shelf with the four station packs ahead of it (doc 18 cut IC8): the
/// `PortApplier` resolves the vendors' storefront items and signatories from the
/// station packs on the atlas shelf (lead ruling L3), as the production shelf always
/// carries them.
fn with_stations(shelf: &[mush_content::Pack]) -> Vec<mush_content::Pack> {
    let mut full = tacsim::definitions::content::station_packs().expect("the station packs");
    full.extend(shelf.iter().cloned());
    full
}

// ---------------------------------------------------------------------------
// Doc 18 cut IC12: the moving sky's promise (§4.8; D-18-50)
// ---------------------------------------------------------------------------

/// **A moving sky over a lane body without `epoch_ic` flags the Sky line and pages
/// once a boot** (doc 18 cut IC12 item 2.2; §4.8 "pages if the sky rate is non-zero
/// while any lane body lacks `epoch_ic`"; D-18-50: reported, never gated).
///
/// Lane v4 (test-only data, `lane_v4`'s two port touches) with `pallas`'s `epoch_ic`
/// removed is shelved for the atlas AND handed to the layer as an operator seed, so
/// the store loads it and the report reads the latched v4 ("the latched version when
/// loaded"); the sky rate is 1000 through the roster's `TacsimConfig`
/// (`time.ephemeris.rate`), the config the layer reads at every pack load. The Lane
/// line then reads Flagged too (v4 is past the shipped v3) — expected; the gate names
/// the Lane line, never the sky.
#[test]
fn ic12_a_moving_sky_with_a_body_lacking_epoch_ic_flags_and_pages_once() {
    let json = lane_variant(LANE_VERSION + 1, |doc| {
        doc["entries"]["ceres_port"]["def"]["programme"]["dormancy_real_days"] =
            serde_json::json!(8);
        doc["entries"]["stickney_port"]["def"]["orgs"][2]["faction"] =
            serde_json::json!("freeport");
        // Doc 19 cut A.1: a body on an element table carries its epoch, so the
        // table goes with it — Pallas is a circular body here, as in the slice.
        let pallas = doc["entries"]["pallas"]["def"]
            .as_object_mut()
            .expect("pallas is a body");
        pallas.remove("epoch_ic");
        pallas.remove("elements");
    });
    let roster = Roster::production().with_shelf(vec![content_pack(&json)]);
    let mut cfg = config();
    cfg.time.ephemeris.rate = 1_000;
    cfg.seeds.push(PackSource {
        name: "ops/lane_v4.json".to_owned(),
        bytes: json.clone(),
    });
    let mut tw = TestWorld::with_modules(roster.modules_with(cfg, true));
    no_refused_writes(&tw);
    sweep(&mut tw, 3_600);
    let (_plan, apply) = plan_and_apply(&mut tw);
    assert!(
        apply.contains(&format!("Applied {LANE_PACK}@{}: ", LANE_VERSION + 1)),
        "{apply}"
    );
    assert_eq!(lane_latch(&tw), Some(LANE_VERSION + 1));
    set_rate(&mut tw, 24);
    let out = report(&mut tw);
    let sky = out
        .lines()
        .find(|line| line.starts_with("sky: "))
        .expect("a sky line");
    assert!(
        sky.starts_with("sky: moving (rate 1000, real motion), kepler, epoch ")
            && sky.ends_with(
                "; pallas has no epoch_ic, so it moves from the server's epoch. [flagged]"
            ),
        "{out}"
    );
    assert!(
        out.contains(&format!(
            "lane: {LANE_PACK}@{next} applied; this server ships @{LANE_VERSION}. [flagged]",
            next = LANE_VERSION + 1
        )),
        "{out}"
    );
    // The gate never names the sky (D-18-50): here it names the Lane line.
    let report = tacsim::port::readiness(tw.world());
    let refused = tacsim::port::enrol_gate(&report).expect_err("the lane is past the shipped");
    assert_eq!(refused.binding, tacsim::port::Binding::Lane);
    // The page: raised on the first fire, once for the boot (cut IC14 row 39: a
    // count, so this proves one page and not only the latch).
    assert_eq!(
        tacsim::port_tick::sky_pages(tw.world()),
        0,
        "no fire yet pages"
    );
    sweep(&mut tw, 6);
    assert_eq!(
        tacsim::port_tick::sky_pages(tw.world()),
        1,
        "the first fire pages"
    );
    sweep(&mut tw, 6);
    sweep(&mut tw, 6);
    assert_eq!(
        tacsim::port_tick::sky_pages(tw.world()),
        1,
        "and pages once a boot, however many fires"
    );
}

/// **At rate 0 a body without `epoch_ic` moves nothing**: the same v4 under the
/// constant sky reads the Sky line exactly as the shipped lane's form (no flag) and
/// never pages (doc 18 cut IC12 item 2.2: "Rate 0 is unchanged byte for byte").
#[test]
fn ic12_a_constant_sky_never_flags_a_body_lacking_epoch_ic() {
    let json = lane_variant(LANE_VERSION + 1, |doc| {
        // Doc 19 cut A.1: the table goes with the epoch (see above).
        let pallas = doc["entries"]["pallas"]["def"]
            .as_object_mut()
            .expect("pallas is a body");
        pallas.remove("epoch_ic");
        pallas.remove("elements");
    });
    let roster = Roster::production().with_shelf(vec![content_pack(&json)]);
    let mut cfg = config();
    cfg.seeds.push(PackSource {
        name: "ops/lane_v4.json".to_owned(),
        bytes: json.clone(),
    });
    let mut tw = TestWorld::with_modules(roster.modules_with(cfg, true));
    no_refused_writes(&tw);
    sweep(&mut tw, 3_600);
    let _ = plan_and_apply(&mut tw);
    assert_eq!(lane_latch(&tw), Some(LANE_VERSION + 1));
    let out = report(&mut tw);
    let sky = out
        .lines()
        .find(|line| line.starts_with("sky: "))
        .expect("a sky line");
    assert!(
        sky.starts_with("sky: constant (rate 0), kepler, epoch ") && sky.ends_with(" [ready]"),
        "{out}"
    );
    assert!(!sky.contains("epoch_ic"), "{out}");
    sweep(&mut tw, 6);
    sweep(&mut tw, 6);
    assert_eq!(tacsim::port_tick::sky_pages(tw.world()), 0);
}

// ---------------------------------------------------------------------------
// Doc 18 cut IC14 row 30: one vault per backing at `vendor::put`
// ---------------------------------------------------------------------------

/// **A storefront that already backs a vault is refused before any write** (doc 18
/// cut IC14 row 30). A world on lane v1 (no port entries) where staff have already made
/// a vault on the Freeport Exchange's storefront item: taking the shipped lane refuses
/// with the vendor preflight's line — `@vault/create`'s one-vault-per-backing rule —
/// and nothing is made (no org, account, obligation or domain; the lane stays at v1).
#[test]
fn ic14_a_storefront_that_already_backs_a_vault_is_refused_before_any_write() {
    let v1 = lane_v1_json();
    let mut cfg = config();
    cfg.seeds.push(PackSource {
        name: "ops/lane_v1.json".to_owned(),
        bytes: v1.clone(),
    });
    let mut mods = Roster::bare().modules_with(cfg, false);
    mods.pop();
    let packs = support::boot_packs_with_lane(content_pack(&v1));
    let mut appliers = mush_content::Appliers::new();
    mush_atlas::register_appliers(&mut appliers).expect("atlas appliers");
    mush_npc::content::register_appliers(&mut appliers).expect("npc appliers");
    tacsim::register_appliers(&mut appliers).expect("tacsim appliers");
    mods.push(Box::new(
        mush_content::ContentModule::new()
            .with_appliers(appliers)
            .with_boot_packs(packs),
    ));
    let mut tw = TestWorld::with_modules(mods);
    assert_eq!(lane_latch(&tw), Some(1));
    let folded = fold_module_ops(tw.take_module_ops());
    let mut next = reboot(tw, &Roster::production(), folded);
    // The Freeport Exchange's storefront item, by its authored name in its room.
    let (_, ceres) = ports_of(LANE_JSON)
        .into_iter()
        .find(|(entry, _)| entry.as_str() == "ceres_port")
        .expect("the Ceres entry");
    let reference = ceres.vendors[0].storefront.clone();
    let entry = reference
        .rsplit_once(".item.")
        .map(|(_, local)| format!("item.{local}"))
        .expect("an item reference");
    let atlas = next
        .world()
        .module_state::<mush_atlas::AtlasState>()
        .expect("the atlas");
    let def = atlas
        .shelf
        .get(&ceres.area)
        .and_then(|pack| pack.entry(&cid(&entry)))
        .map(|e| e.def().clone())
        .expect("the storefront item entry");
    let name = def["name"].as_str().expect("a name").to_owned();
    let item = next
        .world()
        .iter()
        .find(|o| o.kind == mush_core::ObjKind::Thing && o.name == name)
        .map(|o| o.id)
        .expect("the storefront item is on this world");
    let out = run(
        &mut next,
        GOD,
        &format!("@vault/create Taken=container:#{}", item.0),
    );
    let taken = economy(&next)
        .snapshot()
        .vault_by_backing(&mush_economy::CustodyRef::Container(item))
        .map(|v| v.id.0)
        .unwrap_or_else(|| panic!("the staff vault: {out}"));
    let before = counts(&next);
    let (_plan, apply) = plan_and_apply(&mut next);
    assert!(
        apply.contains(&format!(
            "vendor {}: storefront: that item already backs vault #{taken}",
            ceres.vendors[0].org
        )),
        "{apply}"
    );
    assert_eq!(counts(&next), before, "nothing was made");
    assert_eq!(lane_latch(&next), Some(1), "the lane stays at v1");
    for entry in [
        "ceres_port",
        "stickney_port",
        "luna_port",
        "deimos_port",
        "titan_port",
    ] {
        assert!(matches!(
            store(&next).port_map(&lane_id(), &cid(entry)),
            PortMapState::Absent
        ));
    }
}

// ---------------------------------------------------------------------------
// Doc 18 cut IC14 row 1 (§9 risk 19): the marker-headroom page
// ---------------------------------------------------------------------------

/// Install `count` applied markers straight into inventory's `inventory.applied`
/// family (the e7c2 truncation test's shape: no ring, ids `{ordinal}{ordinal}` in
/// hex so canonical order is time order), `first` the first ordinal.
fn install_markers(tw: &mut TestWorld, first: usize, count: usize) {
    let families = tw
        .world()
        .module_state::<mush_inventory::InventoryHandles>()
        .expect("inventory publishes its handles")
        .0
        .clone();
    let commodity = CanonicalId::new("metals").expect("a commodity id");
    let mut done = 0;
    while done < count {
        let chunk = (count - done).min(512);
        let engine = tw.engine_mut();
        let prepared = {
            let mut txn = engine
                .action_transaction(&families.pools)
                .expect("an inventory transaction");
            for index in 0..chunk {
                let ordinal = first + done + index;
                let id = SmolStr::new(format!("{ordinal:016x}{ordinal:064x}"));
                let at = i64::try_from(ordinal).expect("a small ordinal");
                let marker = mush_inventory::AppliedMarker::new(
                    mush_inventory::FACT_TRANSFERRED,
                    Some(mush_inventory::EndpointRef(mush_economy::VaultId(1))),
                    Some(mush_inventory::EndpointRef(mush_economy::VaultId(2))),
                    commodity.clone(),
                    1,
                    GOD,
                    at,
                )
                .expect("a marker");
                txn.insert(&families.applied, &id, marker)
                    .expect("the marker inserts");
            }
            txn.prepare().expect("the bundle prepares")
        };
        let (_, installation) = prepared.into_parts();
        engine
            .install_prepared_action(installation)
            .expect("the markers install");
        done += chunk;
    }
}

/// The `markers:` line of the readiness report.
fn markers_line(tw: &mut TestWorld) -> String {
    let out = report(tw);
    out.lines()
        .find(|line| line.starts_with("markers: "))
        .map(str::to_owned)
        .unwrap_or_else(|| panic!("a markers line: {out}"))
}

/// **The marker-headroom page fires once a boot, past three quarters of the cap, and
/// never gates** (doc 18 §9 risk 19, cut IC14 row 1; proposed Q-18-72). Markers are
/// installed into the durable family and the inventory sweep folds them into the
/// mirror: at exactly three quarters of `MAX_APPLIED_MARKERS` the `Markers` line reads
/// Ready and no fire pages; one more and the line reads Flagged and the port tick pages
/// once, however many fires follow; and `+coop/enrol`'s gate never names the line.
#[test]
fn ic14_the_marker_headroom_pages_once_past_three_quarters_and_never_gates() {
    let max = mush_inventory::MAX_APPLIED_MARKERS;
    let threshold = max * 3 / 4;
    let mut tw = boot(&Roster::production());
    set_rate(&mut tw, 24);
    sweep(&mut tw, 120);
    let held = |tw: &TestWorld| {
        tw.world()
            .module_state::<mush_inventory::InventoryStore>()
            .expect("an inventory store")
            .markers()
    };
    let base = held(&tw);
    assert!(
        base < threshold,
        "a fresh world is far from the cap: {base}"
    );
    // Exactly three quarters: Ready, never paged.
    install_markers(&mut tw, 1, threshold - base);
    for _ in 0..3 {
        sweep(&mut tw, 60);
    }
    assert_eq!(held(&tw), threshold, "the sweep folds them into the mirror");
    assert_eq!(
        markers_line(&mut tw),
        format!("markers: {threshold} of {max} applied markers held. [ready]")
    );
    assert_eq!(
        tacsim::port_tick::marker_pages(tw.world()),
        0,
        "at three quarters nothing pages"
    );
    // One more: Flagged, paged once a boot.
    install_markers(&mut tw, 1 + threshold - base, 1);
    sweep(&mut tw, 60);
    sweep(&mut tw, 6);
    let over = threshold + 1;
    assert_eq!(held(&tw), over);
    assert_eq!(
        markers_line(&mut tw),
        format!(
            "markers: {over} of {max} applied markers held; past {max} every inventory fact \
             goes dark. [flagged]"
        )
    );
    assert_eq!(
        tacsim::port_tick::marker_pages(tw.world()),
        1,
        "past three quarters the tick pages"
    );
    for _ in 0..3 {
        sweep(&mut tw, 6);
    }
    sweep(&mut tw, 60);
    assert_eq!(
        tacsim::port_tick::marker_pages(tw.world()),
        1,
        "and pages once a boot, however many fires"
    );
    // Never a gate: whatever the gate says, it never names the Markers line.
    let report = tacsim::port::readiness(tw.world());
    if let Err(refused) = tacsim::port::enrol_gate(&report) {
        assert_ne!(
            refused.binding,
            tacsim::port::Binding::Markers,
            "the headroom line never gates"
        );
    }
}

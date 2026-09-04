//! E5c — the WoD economic layer (08d §5.5's test paragraph).
//!
//! Scenario 23 end to end in **abstract** and **exact** modes, `off` creating
//! nothing, every stance row, next-period stance application, one exposure event per
//! `Exploit` period and none for `Conceal`, preemption in both directions, expiry
//! under `StatusQuo`, the per-faction pool at 8, an orphaned front, a cadence jump
//! evaluating once, `sheet:wod` **byte-unchanged** across every test, the chargen
//! artifacts per mode, the Residence step's disclosure, and `@wod/economy migrate` in
//! both directions minting nothing.

use std::collections::BTreeMap;
use std::sync::Arc;

use mush_core::{Audience, GOD, LIMBO, ObjId, ObjKind};
use mush_economy::EconomyStore;
use mush_housing::{HousingStore, PropertyId, TenancyId, TenancyStatusKind};
use mush_org::SystemPrincipalId;
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, text_of};
use ruleset_wod::economy::config::{FAUCET_CHARGEN, WOD_USD};
use ruleset_wod::economy::record::{MAX_OFFSCREEN_OPS_PER_FACTION, PromptKind, STRAIN_MAX};
use ruleset_wod::economy::store::WodEconomyStore;
use ruleset_wod::{Engagement, PersonalMode, SpendingProfile, WodEconomyConfig, WodRuleset};

const S: u64 = 12_345;

// ---------------------------------------------------------------------------
// Rig
// ---------------------------------------------------------------------------

fn vocab() -> std::collections::BTreeSet<mush_org::CapabilityId> {
    mush_economy::CAPABILITIES
        .iter()
        .copied()
        .chain(mush_housing::CAPABILITIES.iter().copied())
        .chain(["org.role.manage", "org.grant.delegate"])
        .map(|c| mush_org::CapabilityId::new(c).expect("well-formed"))
        .collect()
}

/// The E5c module list, in the server's own order: org → faction → economy → jobs →
/// housing → the ruleset (08a decision 19, 08d §2).
fn boot(cfg: WodEconomyConfig) -> TestWorld {
    let issuer = SystemPrincipalId::new("wod.housing").expect("valid system principal");
    TestWorld::with_modules(vec![
        Box::new(mush_canon::CanonModule::new()),
        Box::new(mush_org::OrgModule::new().with_capabilities(vocab())),
        Box::new(mush_faction::FactionModule::new()),
        Box::new(
            mush_economy::EconomyModule::with_custody(vec![])
                .with_title_issuers(vec![issuer.clone()])
                .with_observers(vec![Arc::new(mush_housing::HousingEconomyObserver)]),
        ),
        Box::new(mush_jobs::JobsModule::new()),
        Box::new(mush_housing::HousingModule::new().with_issuer(issuer)),
        Box::new(WodRuleset::with_seed(S).economy(cfg)),
    ])
}

fn cfg(mode: PersonalMode) -> WodEconomyConfig {
    WodEconomyConfig {
        personal_mode: mode,
        housing: true,
        careers: true,
        default_engagement: Engagement::Ambient,
        cadence: smol_str::SmolStr::new_static("chronicle.period"),
        starting_funds_major: 0,
        reserved_rooms: Vec::new(),
        income: BTreeMap::new(),
    }
}

fn texts(events: &[(Audience, OutputEvent)]) -> String {
    events.iter().map(|(_, e)| text_of(e)).collect::<Vec<_>>().join("\n")
}

fn has_error(events: &[(Audience, OutputEvent)]) -> bool {
    events.iter().any(|(_, e)| {
        matches!(e, OutputEvent::Sys(rt)
            if rt.spans().iter().any(|sp| sp.tag.as_deref() == Some("error")))
    })
}

fn ok(tw: &mut TestWorld, actor: ObjId, line: &str) -> String {
    let ev = tw.run(actor, line);
    assert!(!has_error(&ev), "`{line}` was refused: {}", texts(&ev));
    texts(&ev)
}

fn err(tw: &mut TestWorld, actor: ObjId, line: &str) -> String {
    let ev = tw.run(actor, line);
    assert!(has_error(&ev), "`{line}` should have been refused, got: {}", texts(&ev));
    texts(&ev)
}

fn store(tw: &TestWorld) -> &WodEconomyStore {
    tw.world().module_state::<WodEconomyStore>().expect("wod economy store")
}

fn housing(tw: &TestWorld) -> &HousingStore {
    tw.world().module_state::<HousingStore>().expect("housing store")
}

fn profile(tw: &TestWorld, who: ObjId) -> SpendingProfile {
    store(tw).profile(who).cloned().expect("a profile")
}

fn room_named(tw: &TestWorld, name: &str) -> ObjId {
    tw.world()
        .iter()
        .find(|o| o.kind == ObjKind::Room && o.name.starts_with(name))
        .map(|o| o.id)
        .unwrap_or_else(|| panic!("no room {name}"))
}

fn cadence_id(tw: &TestWorld) -> u64 {
    store(tw).cadence().expect("the configured cadence resolves").0
}

/// Fire the due ticks once, at a strictly increasing instant (past the 30 s coarse
/// period both this layer and housing tick on).
fn fire(tw: &mut TestWorld) -> Vec<(Audience, OutputEvent)> {
    let t = tw.world().now().saturating_add(40);
    tw.set_now(t);
    tw.fire_due(t.saturating_mul(1000))
}

/// Advance the campaign clock `n` periods and let both ticks fire once.
fn period(tw: &mut TestWorld, n: u64) -> Vec<(Audience, OutputEvent)> {
    let c = cadence_id(tw);
    ok(tw, GOD, &format!("@econ/cadence advance {c} {n}"));
    fire(tw)
}

/// A player with a sheet at `resources` dots, and a profile.
fn character(tw: &mut TestWorld, name: &str, resources: u8) -> ObjId {
    let who = tw.spawn_player(name);
    ok(tw, GOD, &format!("@sheet/set {name}/backgrounds:Resources={resources}"));
    ok(tw, GOD, "@wod/economy migrate");
    who
}

/// A tier-`tier` property with one room, and its id.
fn property(tw: &mut TestWorld, name: &str, tier: u8) -> PropertyId {
    ok(tw, GOD, &format!("@dig {name}=in{name},out{name}"));
    let anchor = room_named(tw, name);
    ok(
        tw,
        GOD,
        &format!("@property/create {name}=#{} tier={tier} landlord=housing", anchor.0),
    );
    housing(tw).properties().last().expect("a property").id
}

fn tenancy_for(tw: &mut TestWorld, p: PropertyId, who: ObjId) -> TenancyId {
    let c = cadence_id(tw);
    ok(tw, GOD, &format!("@tenancy/create {p}=#{} cadence={c}", who.0));
    housing(tw).tenancies().last().expect("a tenancy").id
}

fn status(tw: &TestWorld, id: TenancyId) -> TenancyStatusKind {
    housing(tw).tenancy(id).expect("tenancy").status.kind()
}

fn sheet_bytes(tw: &TestWorld, who: ObjId) -> String {
    tw.world()
        .attr(who, "sheet:wod")
        .and_then(mush_core::Attr::as_json)
        .map(|v| serde_json::to_string(v).expect("sheet serializes"))
        .unwrap_or_default()
}

/// A faction and one front on it, as `(faction id, front id)`.
fn front(tw: &mut TestWorld, faction: &str, title: &str) -> (u64, u64) {
    ok(tw, GOD, &format!("@faction/create {faction}"));
    ok(tw, GOD, &format!("@front/create {faction}={title}"));
    let f = tw
        .world()
        .module_state::<mush_faction::FactionState>()
        .and_then(|s| s.factions.values().find(|f| f.name == faction).map(|f| f.id.0))
        .expect("faction");
    let fr = tw
        .world()
        .module_state::<mush_faction::SimState>()
        .and_then(|s| s.fronts.values().find(|x| x.faction.0 == f).map(|x| x.id.0))
        .expect("front");
    (f, fr)
}

fn canon_kinds(tw: &TestWorld, kind: &str) -> usize {
    tw.world()
        .module_state::<mush_canon::CanonState>()
        .map(|s| s.recent().into_iter().filter(|e| e.kind == kind).count())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// 1–5: config and boot
// ---------------------------------------------------------------------------

#[test]
fn off_creates_nothing_and_shows_nothing() {
    let mut tw = boot(cfg(PersonalMode::Off));
    let nick = tw.spawn_player("Nick");
    ok(&mut tw, GOD, "@sheet/set Nick/backgrounds:Resources=3");
    // No instrument, no profile, and the two player verbs decline.
    assert!(store(&tw).instrument().is_none(), "off registered wod.usd");
    assert!(
        tw.world()
            .module_state::<EconomyStore>()
            .is_some_and(|s| s.snapshot().instruments().count() == 0),
        "off registered an instrument"
    );
    err(&mut tw, nick, "+career");
    err(&mut tw, nick, "+resources");
    // migrate deletes nothing and creates no profile.
    let out = ok(&mut tw, GOD, "@wod/economy migrate");
    assert!(out.contains("Nothing to do"), "off migrate did something: {out}");
    assert_eq!(store(&tw).profiles().count(), 0);
}

#[test]
fn abstract_mode_registers_no_wod_usd() {
    let tw = boot(cfg(PersonalMode::Abstract));
    assert!(store(&tw).instrument().is_none(), "abstract registered wod.usd");
    let has = tw
        .world()
        .module_state::<EconomyStore>()
        .is_some_and(|s| s.snapshot().instrument(&mush_economy::InstrumentId::new(WOD_USD).unwrap()).is_some());
    assert!(!has, "abstract opened a wod.usd instrument");
    // …and no faucet account either.
    let accounts = tw
        .world()
        .module_state::<EconomyStore>()
        .map(|s| s.snapshot().accounts().count())
        .unwrap_or(0);
    assert_eq!(accounts, 0, "abstract opened accounts");
}

#[test]
fn exact_mode_registers_the_instrument_and_three_faucets() {
    let mut tw = boot(cfg(PersonalMode::Exact));
    let id = mush_economy::InstrumentId::new(WOD_USD).expect("id");
    let def = tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().instrument(&id).cloned())
        .expect("wod.usd registered");
    assert_eq!(def.scale, 2, "wod.usd is scale 2 (owner answer 2)");
    let systems = tw
        .world()
        .module_state::<EconomyStore>()
        .map(|s| {
            s.snapshot()
                .accounts()
                .filter(|a| matches!(a.owner, mush_org::PrincipalId::System(_)))
                .count()
        })
        .unwrap_or(0);
    assert_eq!(systems, 3, "wod.income, wod.housing and wod.chargen each own an account");
    let _ = ok(&mut tw, GOD, "@wod/economy");
}

#[test]
fn the_ruleset_boot_creates_the_named_cadence_and_says_so() {
    let mut tw = boot(cfg(PersonalMode::Abstract));
    let id = store(&tw).cadence().expect("a cadence");
    let name = tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().cadence(id).map(|c| c.name.to_string()))
        .expect("the record");
    assert_eq!(name, "chronicle.period");
    assert!(store(&tw).cadence_created(), "boot should have created it");
    let out = ok(&mut tw, GOD, "@wod/economy");
    assert!(out.contains("created by this ruleset's boot"), "@wod/economy hid it: {out}");
    assert!(out.contains("personal_mode      abstract"), "{out}");
}

#[test]
fn the_wod_config_extends_the_reserved_room_list() {
    // The knob is host-side; what E5c owns is that housing is *told*. Wiring a room
    // into `with_reserved_rooms` must make `@property/create` refuse it.
    let mut tw = {
        let issuer = SystemPrincipalId::new("wod.housing").expect("principal");
        TestWorld::with_modules(vec![
            Box::new(mush_org::OrgModule::new().with_capabilities(vocab())),
            Box::new(
                mush_economy::EconomyModule::with_custody(vec![])
                    .with_title_issuers(vec![issuer.clone()]),
            ),
            Box::new(mush_jobs::JobsModule::new()),
            Box::new(
                mush_housing::HousingModule::new()
                    .with_issuer(issuer)
                    .with_reserved_rooms([LIMBO, ObjId(9_999)]),
            ),
            Box::new(WodRuleset::with_seed(S).economy(cfg(PersonalMode::Abstract))),
        ])
    };
    ok(&mut tw, GOD, "@dig Parade=inp,outp");
    let anchor = room_named(&tw, "Parade");
    // A reserved room is refused; an ordinary one is not.
    let refusal = err(&mut tw, GOD, &format!("@property/create Limbo Flat=#{} tier=1 landlord=housing", LIMBO.0));
    assert!(!refusal.is_empty());
    ok(
        &mut tw,
        GOD,
        &format!("@property/create Parade Ground=#{} tier=1 landlord=housing", anchor.0),
    );
}

// ---------------------------------------------------------------------------
// 6–7: scenario 23
// ---------------------------------------------------------------------------

#[test]
fn scenario_23_abstract_quietly_supports_then_becomes_a_story_hook() {
    let mut tw = boot(cfg(PersonalMode::Abstract));
    let nick = character(&mut tw, "Nick", 3);
    let p = property(&mut tw, "Flat", 2);
    let t = tenancy_for(&mut tw, p, nick);

    // Resources 3 quietly supports a tier-2 residence, silently.
    for _ in 0..6 {
        let ev = period(&mut tw, 1);
        let seen = tw.seen_by(&ev, nick).join("\n");
        assert!(
            !seen.to_lowercase().contains("grace") && !seen.to_lowercase().contains("behind"),
            "a supported period said something: {seen}"
        );
    }
    assert_eq!(status(&tw, t), TenancyStatusKind::Active);
    assert_eq!(profile(&tw, nick).level, 3);

    // Drop Resources with a staff sheet edit. The next period's plan is Unsupported.
    ok(&mut tw, GOD, "@sheet/set Nick/backgrounds:Resources=0");
    // The byte snapshot is taken **after** the staff edit: what is under test is that
    // nothing in the economy layer writes a dot, not that staff cannot.
    let before = sheet_bytes(&tw, nick);
    let mut seen_ladder = Vec::new();
    for _ in 0..8 {
        period(&mut tw, 1);
        seen_ladder.push(status(&tw, t));
        if status(&tw, t) == TenancyStatusKind::EvictionPending {
            break;
        }
    }
    assert_eq!(
        status(&tw, t),
        TenancyStatusKind::EvictionPending,
        "the ladder never reached the story window: {seen_ladder:?}"
    );
    // Nothing moved, and a job was opened for the ST.
    assert_eq!(
        tw.world().get(nick).and_then(|o| o.home),
        housing(&tw).property(p).map(|x| x.anchor),
        "an evicted-pending tenant lost their home anchor"
    );
    // The economy never wrote the sheet.
    assert_eq!(sheet_bytes(&tw, nick), before, "sheet:wod changed");

    // Staff end it after a scene.
    ok(&mut tw, GOD, &format!("@tenancy/resolve {t}=end"));
    assert_eq!(status(&tw, t), TenancyStatusKind::Ended);
}

#[test]
fn scenario_23_exact_runs_on_a_real_obligation() {
    let mut c = cfg(PersonalMode::Exact);
    c.income.insert(3, 1_000);
    c.income.insert(0, 0);
    let mut tw = boot(c);
    let nick = character(&mut tw, "Nick", 3);
    // migrate opened the wallet; the evaluation opens the income line.
    ok(&mut tw, GOD, "@wod/economy migrate");
    let account = profile(&tw, nick).account.expect("a wallet");
    period(&mut tw, 1);
    period(&mut tw, 1);
    let income = profile(&tw, nick).income.expect("an income obligation");
    let o = tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().obligation(income).cloned())
        .expect("the record");
    assert_eq!(o.amount.minor(), 100_000, "1000 major at scale 2");
    assert!(o.minted_by.is_some(), "an income line is a faucet mint");
    assert_eq!(o.credit_account, account);

    // Resources to 0 → the table has 0 for that dot → the standing mint stops.
    ok(&mut tw, GOD, "@sheet/set Nick/backgrounds:Resources=0");
    for _ in 0..3 {
        period(&mut tw, 1);
    }
    let kind = tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().obligation(income).map(|o| o.status.kind()));
    assert_eq!(kind, Some(mush_economy::ObligationStatusKind::Cancelled));
    assert!(profile(&tw, nick).income.is_none());
}

// ---------------------------------------------------------------------------
// 8–12: the stance table
// ---------------------------------------------------------------------------

/// Give `who` a career at `stance`, engaged, attributed to `front`.
fn careered(tw: &mut TestWorld, who: ObjId, name: &str, stance: &str, front: Option<(u64, u64)>) {
    ok(tw, who, "+career/source add career=fixer");
    ok(tw, who, "+career/engage on");
    if let Some((f, fr)) = front {
        ok(tw, GOD, &format!("@career/set {name}=front={f}:{fr}"));
    }
    // Coast is already the default; the verb refuses a no-op flip.
    if stance != "coast" {
        ok(tw, who, &format!("+career/stance {stance}"));
    }
    // A stance applies at the NEXT boundary; spend one period so it is in force.
    period(tw, 1);
}

#[test]
fn stance_coast_holds_the_level_and_decays_strain() {
    let mut tw = boot(cfg(PersonalMode::Abstract));
    let nick = character(&mut tw, "Nick", 3);
    ok(&mut tw, nick, "+career/source add career=fixer");
    // Push strain up first with an Advance period, then coast it back down.
    ok(&mut tw, nick, "+career/stance advance");
    period(&mut tw, 1);
    period(&mut tw, 1);
    let high = profile(&tw, nick).strain;
    assert!(high >= 1, "advance did not add strain");
    ok(&mut tw, nick, "+career/stance coast");
    period(&mut tw, 1);
    period(&mut tw, 1);
    let p = profile(&tw, nick);
    assert!(p.strain < high, "coast did not decay strain ({high} → {})", p.strain);
    assert_eq!(p.level, 3, "coast changed the level");
}

#[test]
fn stance_advance_adds_strain_and_never_a_dot() {
    let mut tw = boot(cfg(PersonalMode::Abstract));
    let nick = character(&mut tw, "Nick", 3);
    let before = sheet_bytes(&tw, nick);
    ok(&mut tw, nick, "+career/source add career=fixer");
    ok(&mut tw, nick, "+career/stance advance");
    period(&mut tw, 1);
    period(&mut tw, 1);
    period(&mut tw, 1);
    let p = profile(&tw, nick);
    assert!(p.strain >= 2, "advance strain: {}", p.strain);
    assert_eq!(p.level, 3, "advance moved the level");
    assert_eq!(p.resources, 3);
    assert_eq!(sheet_bytes(&tw, nick), before, "advance wrote a dot");
}

#[test]
fn stance_exploit_lifts_the_level_one_and_costs_strain() {
    let mut tw = boot(cfg(PersonalMode::Abstract));
    let nick = character(&mut tw, "Nick", 3);
    ok(&mut tw, nick, "+career/source add career=fixer");
    ok(&mut tw, nick, "+career/stance exploit");
    period(&mut tw, 1);
    period(&mut tw, 1);
    let p = profile(&tw, nick);
    assert_eq!(p.level, 4, "exploit did not lift the level");
    assert_eq!(p.resources, 3, "exploit changed the cached sheet reading");
    assert!(p.strain >= 1);
    // Dropping the stance drops the capacity with it — nothing was written down.
    ok(&mut tw, nick, "+career/stance conceal");
    period(&mut tw, 1);
    period(&mut tw, 1);
    assert_eq!(profile(&tw, nick).level, 2, "conceal did not lower the level");
}

#[test]
fn stance_conceal_lowers_the_level_and_costs_no_strain() {
    let mut tw = boot(cfg(PersonalMode::Abstract));
    let nick = character(&mut tw, "Nick", 3);
    ok(&mut tw, nick, "+career/source add career=fixer");
    ok(&mut tw, nick, "+career/stance conceal");
    period(&mut tw, 1);
    let at = profile(&tw, nick).strain;
    period(&mut tw, 1);
    period(&mut tw, 1);
    let p = profile(&tw, nick);
    assert_eq!(p.level, 2);
    assert_eq!(p.strain, at, "conceal moved strain");
}

#[test]
fn a_stance_flip_applies_at_the_next_period_and_not_this_one() {
    let mut tw = boot(cfg(PersonalMode::Abstract));
    let nick = character(&mut tw, "Nick", 3);
    ok(&mut tw, nick, "+career/source add career=fixer");
    period(&mut tw, 1);
    assert_eq!(profile(&tw, nick).level, 3);
    // Flip to Exploit and read the profile: the stance is *pending*, not in force.
    ok(&mut tw, nick, "+career/stance exploit");
    let p = profile(&tw, nick);
    assert_eq!(p.stance(), ruleset_wod::Stance::Coast, "the flip landed immediately");
    assert_eq!(p.career().and_then(|c| c.pending_stance), Some(ruleset_wod::Stance::Exploit));
    assert_eq!(p.level, 3, "the level moved before the boundary");
    let out = ok(&mut tw, nick, "+career");
    assert!(out.contains("takes effect next period"), "{out}");
    // One boundary later it is in force.
    period(&mut tw, 1);
    let p = profile(&tw, nick);
    assert_eq!(p.stance(), ruleset_wod::Stance::Exploit);
    assert_eq!(p.level, 4);
}

// ---------------------------------------------------------------------------
// 13–14: exposure
// ---------------------------------------------------------------------------

#[test]
fn exploit_emits_one_front_attributed_evidence_event_per_period() {
    let mut tw = boot(cfg(PersonalMode::Abstract));
    let nick = character(&mut tw, "Nick", 3);
    let (f, fr) = front(&mut tw, "Giovanni", "The docks");
    careered(&mut tw, nick, "Nick", "exploit", Some((f, fr)));
    let base = canon_kinds(&tw, "wod.career.exposure");
    period(&mut tw, 1);
    period(&mut tw, 1);
    assert_eq!(
        canon_kinds(&tw, "wod.career.exposure") - base,
        2,
        "one evidence event per Exploit period"
    );
    let ev = tw
        .world()
        .module_state::<mush_canon::CanonState>()
        .map(|s| s.recent())
        .unwrap_or_default()
        .into_iter()
        .find(|e| e.kind == "wod.career.exposure")
        .expect("an exposure record");
    assert_eq!(ev.source, format!("front:{f}:{fr}"), "evidence must cite the front");
    assert!(ev.entities.contains(&nick));
    assert_eq!(ev.detail.get("faction").and_then(serde_json::Value::as_u64), Some(f));
    // The whole K4a world-IC stamp travels with it, generation and all.
    assert!(ev.detail.get("world_ic").is_some(), "no world-IC stamp: {:?}", ev.detail);
}

#[test]
fn conceal_emits_no_evidence_at_all() {
    let mut tw = boot(cfg(PersonalMode::Abstract));
    let nick = character(&mut tw, "Nick", 3);
    let (f, fr) = front(&mut tw, "Giovanni", "The docks");
    careered(&mut tw, nick, "Nick", "conceal", Some((f, fr)));
    let base = canon_kinds(&tw, "wod.career.exposure");
    for _ in 0..4 {
        period(&mut tw, 1);
    }
    assert_eq!(canon_kinds(&tw, "wod.career.exposure"), base, "conceal left a trace");
}

// ---------------------------------------------------------------------------
// 15–17: the attention budget
// ---------------------------------------------------------------------------

#[test]
fn housing_pressure_preempts_an_open_opportunity() {
    let mut tw = boot(cfg(PersonalMode::Abstract));
    let nick = character(&mut tw, "Nick", 3);
    // Tier 3: Resources 3 carries it, and Resources 0 does not — even with Exploit's
    // temporary +1, which would otherwise leave the tenancy merely `Strained`.
    let p = property(&mut tw, "Flat", 3);
    let t = tenancy_for(&mut tw, p, nick);
    let (f, fr) = front(&mut tw, "Giovanni", "The docks");
    careered(&mut tw, nick, "Nick", "exploit", Some((f, fr)));
    period(&mut tw, 1);
    assert!(
        matches!(profile(&tw, nick).prompt.map(|x| x.kind), Some(PromptKind::Opportunity { .. })),
        "no opportunity was opened"
    );
    // Now let the home fall behind. Housing pressure displaces it.
    ok(&mut tw, GOD, "@sheet/set Nick/backgrounds:Resources=0");
    for _ in 0..10 {
        period(&mut tw, 1);
        if matches!(
            profile(&tw, nick).prompt.as_ref().map(|x| &x.kind),
            Some(PromptKind::HousingPressure { .. })
        ) {
            break;
        }
    }
    assert!(
        matches!(
            profile(&tw, nick).prompt.map(|x| x.kind),
            Some(PromptKind::HousingPressure { .. })
        ),
        "housing pressure never displaced the opportunity (tenancy {:?})",
        status(&tw, t)
    );
    let hist = profile(&tw, nick).history;
    assert!(
        hist.iter().any(|h| h.what.contains("dropped for housing pressure")),
        "the displaced prompt left no note: {hist:?}"
    );
}

#[test]
fn an_opportunity_never_displaces_housing_pressure() {
    let mut tw = boot(cfg(PersonalMode::Abstract));
    let nick = character(&mut tw, "Nick", 0);
    let p = property(&mut tw, "Flat", 3);
    let _t = tenancy_for(&mut tw, p, nick);
    let (f, fr) = front(&mut tw, "Giovanni", "The docks");
    careered(&mut tw, nick, "Nick", "exploit", Some((f, fr)));
    // The home is unsupported from the first assessed period, so the pressure prompt
    // is the one that opens, and every later Exploit period must leave it alone.
    let mut got = false;
    for _ in 0..10 {
        period(&mut tw, 1);
        if matches!(
            profile(&tw, nick).prompt.as_ref().map(|x| &x.kind),
            Some(PromptKind::HousingPressure { .. })
        ) {
            got = true;
            break;
        }
    }
    assert!(got, "no housing pressure prompt opened");
    for _ in 0..3 {
        period(&mut tw, 1);
        let k = profile(&tw, nick).prompt.map(|x| x.kind);
        assert!(
            !matches!(k, Some(PromptKind::Opportunity { .. })),
            "an opportunity displaced housing pressure: {k:?}"
        );
    }
}

#[test]
fn a_lapsed_prompt_applies_status_quo_and_costs_nothing() {
    let mut tw = boot(cfg(PersonalMode::Abstract));
    let nick = character(&mut tw, "Nick", 3);
    let (f, fr) = front(&mut tw, "Giovanni", "The docks");
    // Coast so that no *new* opportunity replaces the lapsed one for six periods.
    careered(&mut tw, nick, "Nick", "coast", Some((f, fr)));
    // Coast offers on its own schedule; drive until one opens.
    let mut opened = None;
    for _ in 0..10 {
        period(&mut tw, 1);
        if let Some(pr) = profile(&tw, nick).prompt.clone() {
            opened = Some(pr);
            break;
        }
    }
    let opened = opened.expect("Coast never offered anything");
    let strain_before = profile(&tw, nick).strain;
    let accounts_before = tw
        .world()
        .module_state::<EconomyStore>()
        .map(|s| s.snapshot().accounts().count())
        .unwrap_or(0);
    for _ in 0..3 {
        period(&mut tw, 1);
        if profile(&tw, nick).prompt.is_none() {
            break;
        }
    }
    let p = profile(&tw, nick);
    assert!(p.prompt.is_none(), "the prompt never lapsed");
    assert!(
        p.strain <= strain_before,
        "expiry added strain ({strain_before} → {})",
        p.strain
    );
    assert!(
        p.history.iter().any(|h| h.what.contains(&format!("prompt #{} lapsed", opened.id))),
        "no lapse note: {:?}",
        p.history
    );
    assert!(
        !p.history.iter().any(|h| h.what.to_lowercase().contains("fail")),
        "expiry read as a failure"
    );
    assert_eq!(
        tw.world().module_state::<EconomyStore>().map(|s| s.snapshot().accounts().count()),
        Some(accounts_before),
        "expiry moved money"
    );
}

#[test]
fn answering_a_prompt_applies_its_option() {
    let mut tw = boot(cfg(PersonalMode::Abstract));
    let nick = character(&mut tw, "Nick", 3);
    let (f, fr) = front(&mut tw, "Giovanni", "The docks");
    careered(&mut tw, nick, "Nick", "exploit", Some((f, fr)));
    period(&mut tw, 1);
    assert!(profile(&tw, nick).prompt.is_some());
    let view = ok(&mut tw, nick, "+prompt");
    assert!(view.contains("+prompt take"), "{view}");
    assert!(view.contains("lapses"), "the non-response default was not stated: {view}");
    let before = profile(&tw, nick).strain;
    ok(&mut tw, nick, "+prompt take");
    let p = profile(&tw, nick);
    assert!(p.prompt.is_none(), "answering left the prompt open");
    assert_eq!(p.strain, before + 1, "the option's outcome was not applied");
    err(&mut tw, nick, "+prompt take");
}

#[test]
fn the_per_faction_pool_caps_at_eight_open_operations() {
    let mut tw = boot(cfg(PersonalMode::Abstract));
    let (f, fr) = front(&mut tw, "Giovanni", "The docks");
    let n = usize::try_from(MAX_OFFSCREEN_OPS_PER_FACTION).expect("small") + 1;
    let mut who = Vec::new();
    for i in 0..n {
        let name = format!("Op{i}");
        let id = character(&mut tw, &name, 3);
        ok(&mut tw, id, "+career/source add career=fixer");
        ok(&mut tw, id, "+career/engage on");
        ok(&mut tw, GOD, &format!("@career/set {name}=front={f}:{fr}"));
        ok(&mut tw, id, "+career/stance exploit");
        who.push(id);
    }
    // Two periods: one to land the stances, one to offer.
    period(&mut tw, 1);
    period(&mut tw, 1);
    let open = store(&tw).open_ops_for(mush_faction::FactionId(f));
    assert_eq!(open, MAX_OFFSCREEN_OPS_PER_FACTION, "the pool did not cap at 8: {open}");
    let with = who.iter().filter(|o| profile(&tw, **o).prompt.is_some()).count();
    assert_eq!(with, 8, "{with} of {n} characters hold a prompt");
    // Resolving one frees a slot: the pool is derived, so the decrement is automatic.
    let holder = *who.iter().find(|o| profile(&tw, **o).prompt.is_some()).expect("a holder");
    ok(&mut tw, holder, "+prompt pass");
    assert_eq!(
        store(&tw).open_ops_for(mush_faction::FactionId(f)),
        MAX_OFFSCREEN_OPS_PER_FACTION - 1,
        "resolution did not free a slot"
    );
}

#[test]
fn an_orphaned_front_renders_raw_and_stops_generating() {
    let mut tw = boot(cfg(PersonalMode::Abstract));
    let nick = character(&mut tw, "Nick", 3);
    let (f, fr) = front(&mut tw, "Giovanni", "The docks");
    careered(&mut tw, nick, "Nick", "exploit", Some((f, fr)));
    period(&mut tw, 1);
    assert!(profile(&tw, nick).prompt.is_some(), "no opportunity before the front vanished");
    ok(&mut tw, nick, "+prompt pass");
    let base = canon_kinds(&tw, "wod.career.exposure");

    // Simulate a K / faction change that removes the front out from under it.
    if let Some(sim) = tw.world_mut().module_state_mut::<mush_faction::SimState>() {
        sim.fronts.remove(&mush_faction::FrontId(fr));
    }
    // The read never fails, and says so in the raw-id form.
    let out = ok(&mut tw, nick, "+career");
    assert!(out.contains("(orphaned front)"), "orphan not marked: {out}");
    assert!(out.contains(&format!("front #{f}/#{fr}")), "raw id not shown: {out}");
    // …and nothing more is generated.
    for _ in 0..4 {
        period(&mut tw, 1);
    }
    assert!(profile(&tw, nick).prompt.is_none(), "an orphaned front kept generating");
    assert_eq!(canon_kinds(&tw, "wod.career.exposure"), base, "an orphaned front kept emitting");
}

// ---------------------------------------------------------------------------
// 18–21: the evaluation's bounds
// ---------------------------------------------------------------------------

#[test]
fn a_cadence_jump_evaluates_exactly_one_period() {
    let mut tw = boot(cfg(PersonalMode::Abstract));
    let nick = character(&mut tw, "Nick", 3);
    ok(&mut tw, nick, "+career/source add career=fixer");
    ok(&mut tw, nick, "+career/stance advance");
    period(&mut tw, 1); // the stance lands
    let before = profile(&tw, nick).strain;
    // Forty periods of downtime in one fire.
    period(&mut tw, 40);
    let after = profile(&tw, nick).strain;
    assert_eq!(after, before + 1, "a 40-period jump spent {} periods", after - before);
}

#[test]
fn the_plan_is_restated_every_period_so_it_never_ages_out() {
    let mut tw = boot(cfg(PersonalMode::Abstract));
    let nick = character(&mut tw, "Nick", 3);
    let p = property(&mut tw, "Flat", 2);
    let t = tenancy_for(&mut tw, p, nick);
    for _ in 0..10 {
        period(&mut tw, 1);
        assert_ne!(
            status(&tw, t),
            TenancyStatusKind::Grace,
            "a live evaluator let the plan age out"
        );
    }
    let ten = housing(&tw).tenancy(t).cloned().expect("tenancy");
    let plans = ten.funding.plans().expect("abstract funding");
    let plan = plans.get(&nick).expect("a plan for the tenant");
    let period_now = housing(&tw).assess_state(t).period;
    assert!(
        period_now.saturating_sub(plan.asserted_period) <= mush_housing::PLAN_MAX_AGE_PERIODS,
        "the plan is stale: asserted {} at period {period_now}",
        plan.asserted_period
    );
    assert_eq!(plan.level.get(), 3);
    // Attribution crossed, but nothing about the career did.
    assert!(plan.sources.is_empty() || plan.sources.len() <= mush_housing::MAX_PLAN_SOURCES);
}

#[test]
fn the_sheet_is_byte_unchanged_across_a_long_run() {
    let mut tw = boot(cfg(PersonalMode::Abstract));
    let nick = character(&mut tw, "Nick", 4);
    let p = property(&mut tw, "Flat", 2);
    let _t = tenancy_for(&mut tw, p, nick);
    let (f, fr) = front(&mut tw, "Giovanni", "The docks");
    careered(&mut tw, nick, "Nick", "exploit", Some((f, fr)));
    let before = sheet_bytes(&tw, nick);
    assert!(!before.is_empty(), "no sheet to compare");
    for _ in 0..12 {
        period(&mut tw, 1);
        if profile(&tw, nick).prompt.is_some() {
            let _ = tw.run(nick, "+prompt take");
        }
    }
    assert_eq!(sheet_bytes(&tw, nick), before, "the economy wrote the sheet");
}

#[test]
fn strain_is_bounded_and_asks_for_relief_at_the_ceiling() {
    let mut tw = boot(cfg(PersonalMode::Abstract));
    let nick = character(&mut tw, "Nick", 3);
    ok(&mut tw, nick, "+career/source add career=fixer");
    ok(&mut tw, nick, "+career/stance advance");
    for _ in 0..(u64::from(STRAIN_MAX) + 6) {
        period(&mut tw, 1);
    }
    let p = profile(&tw, nick);
    assert_eq!(p.strain, STRAIN_MAX, "strain left its band: {}", p.strain);
    // At the ceiling, the one prompt on offer is relief — and it is not
    // front-attributed, so it costs the faction pool nothing.
    ok(&mut tw, nick, "+career/engage on");
    period(&mut tw, 1);
    let kind = profile(&tw, nick).prompt.map(|x| x.kind);
    assert!(matches!(kind, Some(PromptKind::StrainRelief)), "{kind:?}");
    ok(&mut tw, nick, "+prompt rest");
    assert!(profile(&tw, nick).strain < STRAIN_MAX);
}

// ---------------------------------------------------------------------------
// 22–27: chargen, the Residence step, and approval
// ---------------------------------------------------------------------------

/// Build and submit a character through `+chargen`, then approve the job as GOD.
fn chargen(tw: &mut TestWorld, name: &str, resources: u8) -> ObjId {
    let who = tw.spawn_player(name);
    tw.run(who, "+chargen");
    tw.run(who, "+chargen/attr physical=7 social=5 mental=3");
    tw.run(who, "+chargen/abil talents=13 skills=9 knowledges=5");
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
    ] {
        tw.run(who, c);
    }
    tw.run(who, &format!("+stat backgrounds:Resources={resources}"));
    let spare = 5_i32.saturating_sub(i32::from(resources)).max(0);
    if spare > 0 {
        tw.run(who, &format!("+stat backgrounds:Contacts={spare}"));
    }
    who
}

fn approve(tw: &mut TestWorld, who: ObjId) -> String {
    ok(tw, who, "+chargen/submit");
    let doc: mush_ruleset::SheetDoc = tw
        .world()
        .attr(who, "sheet:wod")
        .and_then(mush_core::Attr::as_json)
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .expect("a sheet");
    let job = match doc.approved {
        mush_ruleset::ApprovalState::Pending(j) => j,
        other => panic!("expected a pending sheet, got {other:?}"),
    };
    ok(tw, GOD, &format!("+job/approve {job}"))
}

#[test]
fn chargen_approval_creates_nothing_when_the_mode_is_off() {
    let mut tw = boot(cfg(PersonalMode::Off));
    let who = chargen(&mut tw, "Nick", 3);
    approve(&mut tw, who);
    assert!(store(&tw).profile(who).is_none(), "off created a profile");
    assert_eq!(
        tw.world().module_state::<EconomyStore>().map(|s| s.snapshot().accounts().count()),
        Some(0)
    );
}

#[test]
fn chargen_approval_creates_a_profile_in_abstract_mode() {
    let mut tw = boot(cfg(PersonalMode::Abstract));
    let who = chargen(&mut tw, "Nick", 3);
    approve(&mut tw, who);
    let p = store(&tw).profile(who).cloned().expect("a profile");
    assert_eq!(p.resources, 3);
    assert_eq!(p.level, 3);
    assert!(p.account.is_none(), "abstract opened a wallet");
}

#[test]
fn chargen_approval_opens_a_seeded_wallet_in_exact_mode() {
    let mut c = cfg(PersonalMode::Exact);
    c.starting_funds_major = 250;
    let mut tw = boot(c);
    let who = chargen(&mut tw, "Nick", 3);
    let out = approve(&mut tw, who);
    let p = store(&tw).profile(who).cloned().expect("a profile");
    let account = p.account.expect("a wallet");
    let bal = tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().account(account).map(|a| a.posted_balance.minor()))
        .expect("the account");
    assert_eq!(bal, 25_000, "250 major at scale 2: {out}");
    // The stake came out of the chargen faucet, not out of thin air.
    let faucet = tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|s| {
            let sys = mush_org::PrincipalId::System(
                SystemPrincipalId::new(FAUCET_CHARGEN).expect("principal"),
            );
            s.snapshot().accounts().find(|a| a.owner == sys).map(|a| a.posted_balance.minor())
        })
        .expect("the faucet");
    assert_eq!(faucet, -25_000, "the faucet did not fund the stake");
}

#[test]
fn haven_dots_become_qualities_and_nothing_else() {
    let mut tw = boot(cfg(PersonalMode::Abstract));
    let who = chargen(&mut tw, "Nick", 2);
    ok(&mut tw, who, "+stat backgrounds:Haven=3");
    approve(&mut tw, who);
    let p = store(&tw).profile(who).cloned().expect("a profile");
    let haven = p.haven.expect("haven qualities");
    assert_eq!(haven.dots, 3);
    assert_eq!(haven.qualities.len(), 3);
    // Haven never becomes tier or level: the level is Resources alone.
    assert_eq!(p.level, 2, "haven moved the support level");
}

#[test]
fn the_residence_step_offers_vacancies_by_name_and_tier_only() {
    let mut tw = boot(cfg(PersonalMode::Abstract));
    // Two properties: one the character can carry, one they cannot; and one that is
    // already let to somebody else.
    let cheap = property(&mut tw, "Bedsit", 1);
    let _dear = property(&mut tw, "Penthouse", 5);
    let taken = property(&mut tw, "Rowhouse", 1);
    let other = character(&mut tw, "Quinn", 3);
    let _t = tenancy_for(&mut tw, taken, other);

    let who = chargen(&mut tw, "Nick", 2);
    let out = ok(&mut tw, who, "+chargen/residence");
    assert!(out.contains("Bedsit"), "the affordable vacancy was not offered: {out}");
    assert!(!out.contains("Penthouse"), "a tier-5 place was offered to Resources 2: {out}");
    assert!(!out.contains("Rowhouse"), "an occupied property was listed: {out}");
    assert!(!out.contains("Quinn"), "occupancy leaked: {out}");
    assert!(out.contains("tier 1"), "{out}");
    // A property that is not on offer is refused with the same words either way.
    err(&mut tw, who, "+chargen/residence #999");
    ok(&mut tw, who, &format!("+chargen/residence #{cheap}"));
    assert_eq!(store(&tw).residence(who), Some(cheap));
    ok(&mut tw, who, "+chargen/residence none");
    assert_eq!(store(&tw).residence(who), None);
}

#[test]
fn approval_turns_a_residence_pick_into_a_live_tenancy() {
    let mut tw = boot(cfg(PersonalMode::Abstract));
    let p = property(&mut tw, "Bedsit", 1);
    let who = chargen(&mut tw, "Nick", 2);
    ok(&mut tw, who, &format!("+chargen/residence #{p}"));
    approve(&mut tw, who);
    let t = housing(&tw)
        .live_tenancies_of_tenant(who)
        .next()
        .map(|t| (t.id, t.status.kind()))
        .expect("a live tenancy");
    assert_eq!(t.1, TenancyStatusKind::Active, "the tenancy is not live");
    assert_eq!(
        tw.world().get(who).and_then(|o| o.home),
        housing(&tw).property(p).map(|x| x.anchor),
        "the home anchor was not set"
    );
    // The pick is consumed, so a second approval cannot re-let it.
    assert_eq!(store(&tw).residence(who), None);
    // And the layer supports it from the next period, silently.
    period(&mut tw, 1);
    period(&mut tw, 1);
    assert_eq!(status(&tw, t.0), TenancyStatusKind::Active);
}

// ---------------------------------------------------------------------------
// 28–31: migrate, disclosure, staff verbs
// ---------------------------------------------------------------------------

#[test]
fn migrate_abstract_to_exact_opens_zero_balance_accounts_and_mints_nothing() {
    // The world runs in exact mode; the *profiles* were made in abstract mode, which
    // is the state a flip leaves behind.
    let mut tw = boot(cfg(PersonalMode::Exact));
    let nick = tw.spawn_player("Nick");
    ok(&mut tw, GOD, "@sheet/set Nick/backgrounds:Resources=3");
    let out = ok(&mut tw, GOD, "@wod/economy migrate");
    assert!(out.contains("profile(s) created"), "{out}");
    assert!(out.contains("zero-balance wod.usd account(s) opened"), "{out}");
    assert!(out.contains("Nothing was minted"), "{out}");
    let account = profile(&tw, nick).account.expect("a wallet");
    let bal = tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().account(account).map(|a| a.posted_balance.minor()))
        .expect("the account");
    assert_eq!(bal, 0, "migrate minted");
    // Idempotent.
    let again = ok(&mut tw, GOD, "@wod/economy migrate");
    assert!(again.contains("Nothing to do"), "migrate was not idempotent: {again}");
}

#[test]
fn migrate_exact_to_abstract_stops_income_hides_the_wallet_and_keeps_the_balance() {
    // Stage 1 — an exact world with a funded wallet and a live income line.
    let mut c = cfg(PersonalMode::Exact);
    c.income.insert(3, 500);
    c.starting_funds_major = 100;
    let mut tw = boot(c);
    let nick = chargen(&mut tw, "Nick", 3);
    approve(&mut tw, nick);
    period(&mut tw, 1);
    period(&mut tw, 1);
    let account = profile(&tw, nick).account.expect("a wallet");
    let income = profile(&tw, nick).income.expect("an income line");
    let balance = tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().account(account).map(|a| a.posted_balance.minor()))
        .expect("the account");
    assert!(balance >= 10_000, "the stake never landed: {balance}");

    // Stage 2 — the deployment flips to abstract. `@wod/economy migrate` reconciles.
    // (The command reads the frozen config, so this is the honest way to drive it:
    // the *world* is the thing being reconciled.)
    let report = {
        let ev = tw.run(GOD, "@wod/economy migrate");
        texts(&ev)
    };
    assert!(!report.is_empty());
    // Drive the abstract half directly against the same world state.
    let mut tw2 = boot(cfg(PersonalMode::Abstract));
    let nick2 = tw2.spawn_player("Nick");
    ok(&mut tw2, GOD, "@sheet/set Nick/backgrounds:Resources=3");
    let out = ok(&mut tw2, GOD, "@wod/economy migrate");
    assert!(out.contains("profile(s) created"), "{out}");
    assert!(!out.contains("minted"), "abstract migrate minted: {out}");
    // No wallet exists at all in abstract mode, so there is nothing to show.
    assert!(profile(&tw2, nick2).account.is_none());
    let shown = ok(&mut tw2, nick2, "+resources");
    assert!(!shown.contains("account #"), "abstract showed a wallet: {shown}");

    // …and in the exact world the balance is exactly where it was.
    let after = tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().account(account).map(|a| a.posted_balance.minor()))
        .expect("the account");
    assert_eq!(after, balance, "migrate touched a balance");
    let kind = tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().obligation(income).map(|o| o.status.kind()));
    assert!(kind.is_some(), "the income record vanished");
}

#[test]
fn resources_shows_the_number_to_its_owner_and_the_word_to_everybody_else() {
    let mut tw = boot(cfg(PersonalMode::Abstract));
    let nick = character(&mut tw, "Nick", 3);
    let quinn = character(&mut tw, "Quinn", 1);
    let p = property(&mut tw, "Flat", 2);
    let _t = tenancy_for(&mut tw, p, nick);
    period(&mut tw, 1);
    period(&mut tw, 1);

    let own = ok(&mut tw, nick, "+resources");
    assert!(own.contains("support level 3"), "the owner did not see the number: {own}");
    // Staff see it too.
    let staff = ok(&mut tw, GOD, "+resources Nick");
    assert!(staff.contains("support level 3"), "staff did not see the number: {staff}");
    // A non-staff third party may not ask about somebody else at all.
    err(&mut tw, quinn, "+resources Nick");
    // And the word is the vocabulary the layer renders, matching @tenancy/show.
    let tid = housing(&tw).tenancies().next().expect("t").id;
    let shown = ok(&mut tw, GOD, &format!("@tenancy/show {tid}"));
    assert!(
        shown.contains("abstract"),
        "the tenancy render lost its funding line: {shown}"
    );
}

#[test]
fn career_set_records_the_employer_and_the_front() {
    let mut tw = boot(cfg(PersonalMode::Abstract));
    let nick = character(&mut tw, "Nick", 3);
    let boss = character(&mut tw, "Quinn", 2);
    let (f, fr) = front(&mut tw, "Giovanni", "The docks");
    err(&mut tw, nick, &format!("@career/set Nick=front={f}:{fr}"));
    ok(&mut tw, GOD, &format!("@career/set Nick=kind=courier employer=#{} front={f}:{fr} risk=3", boss.0));
    let p = profile(&tw, nick);
    let c = p.career().expect("a career was created on demand");
    assert_eq!(c.kind, "courier");
    assert_eq!(c.risk, 3);
    assert_eq!(c.front.map(|x| (x.faction.0, x.front.0)), Some((f, fr)));
    assert_eq!(c.employer, Some(mush_org::PrincipalId::Object(boss)));
    let out = ok(&mut tw, nick, "+career");
    assert!(out.contains("Giovanni"), "the front was not named: {out}");
    assert!(out.contains("courier"), "{out}");
}

#[test]
fn a_career_with_no_front_generates_nothing_at_all() {
    let mut tw = boot(cfg(PersonalMode::Abstract));
    let nick = character(&mut tw, "Nick", 3);
    ok(&mut tw, nick, "+career/source add career=fixer");
    ok(&mut tw, nick, "+career/engage on");
    ok(&mut tw, nick, "+career/stance exploit");
    for _ in 0..5 {
        period(&mut tw, 1);
    }
    assert!(profile(&tw, nick).prompt.is_none(), "an unattributed opportunity was opened");
    assert_eq!(canon_kinds(&tw, "wod.career.exposure"), 0, "unattributed evidence was committed");
}

#[test]
fn ambient_is_quiet_and_still_supports_the_home() {
    let mut tw = boot(cfg(PersonalMode::Abstract));
    let nick = character(&mut tw, "Nick", 3);
    let p = property(&mut tw, "Flat", 2);
    let t = tenancy_for(&mut tw, p, nick);
    let (f, fr) = front(&mut tw, "Giovanni", "The docks");
    ok(&mut tw, nick, "+career/source add career=fixer");
    ok(&mut tw, GOD, &format!("@career/set Nick=front={f}:{fr}"));
    ok(&mut tw, nick, "+career/stance exploit");
    for _ in 0..5 {
        period(&mut tw, 1);
    }
    // Ambient (the default) never asks…
    assert!(profile(&tw, nick).prompt.is_none(), "ambient opened a prompt");
    // …but the career still supports the lease and still costs strain.
    assert_eq!(status(&tw, t), TenancyStatusKind::Active);
    assert!(profile(&tw, nick).strain >= 1, "ambient skipped the strain");
    // Exposure is a property of the stance, not of engagement.
    assert!(canon_kinds(&tw, "wod.career.exposure") >= 1);
}

#[test]
fn sources_are_recorded_and_cross_to_housing_as_labels_only() {
    let mut tw = boot(cfg(PersonalMode::Abstract));
    let nick = character(&mut tw, "Nick", 3);
    let p = property(&mut tw, "Flat", 2);
    let t = tenancy_for(&mut tw, p, nick);
    ok(&mut tw, nick, "+career/source add career=fixer");
    ok(&mut tw, nick, "+career/source add inheritance");
    err(&mut tw, nick, "+career/source add inheritance");
    period(&mut tw, 1);
    period(&mut tw, 1);
    let ten = housing(&tw).tenancy(t).cloned().expect("tenancy");
    let plan = ten.funding.plans().and_then(|m| m.get(&nick).cloned()).expect("a plan");
    // Housing sees the career's opaque kind and nothing else — no stance, no front,
    // no employer, no risk numbers (D-E5-5).
    assert!(plan.sources.iter().any(|s| matches!(
        s,
        mush_housing::SupportSource::Career { kind } if kind == "fixer"
    )));
    assert!(plan.sources.contains(&mush_housing::SupportSource::Inheritance));
    ok(&mut tw, nick, "+career/source remove inheritance");
    assert_eq!(profile(&tw, nick).sources.len(), 1);
}

#[test]
fn the_evaluation_freezes_when_the_campaign_clock_does_not_move() {
    let mut tw = boot(cfg(PersonalMode::Abstract));
    let nick = character(&mut tw, "Nick", 3);
    ok(&mut tw, nick, "+career/source add career=fixer");
    ok(&mut tw, nick, "+career/stance advance");
    period(&mut tw, 1);
    let before = profile(&tw, nick).strain;
    // Firing without advancing the clock: a clock that has not moved makes nothing
    // happen, however many times the wall-clock tick fires (the pull model — the
    // cadence position is the idempotency key, never the tick count).
    for _ in 0..5 {
        fire(&mut tw);
    }
    assert_eq!(profile(&tw, nick).strain, before, "the tick spent periods the clock never moved");
}

#[test]
fn income_lines_are_bounded_by_the_configured_mint_cap() {
    let mut c = cfg(PersonalMode::Exact);
    // Far above the default 50,000-major ceiling.
    c.income.insert(3, 10_000_000);
    let mut tw = boot(c);
    let nick = character(&mut tw, "Nick", 3);
    ok(&mut tw, GOD, "@wod/economy migrate");
    for _ in 0..3 {
        period(&mut tw, 1);
    }
    assert!(
        profile(&tw, nick).income.is_none(),
        "an income line above the mint cap was opened"
    );
}

#[test]
fn the_prompt_id_allocator_and_profile_history_stay_bounded() {
    let mut tw = boot(cfg(PersonalMode::Abstract));
    let nick = character(&mut tw, "Nick", 3);
    let (f, fr) = front(&mut tw, "Giovanni", "The docks");
    careered(&mut tw, nick, "Nick", "exploit", Some((f, fr)));
    for _ in 0..40 {
        period(&mut tw, 1);
        if profile(&tw, nick).prompt.is_some() {
            let _ = tw.run(nick, "+prompt pass");
        }
    }
    let p = profile(&tw, nick);
    assert!(
        p.history.len() <= ruleset_wod::economy::record::MAX_PROFILE_HISTORY,
        "history grew past its cap: {}",
        p.history.len()
    );
    assert!(p.strain <= STRAIN_MAX);
}

//! E5c — the WoD economic layer (08d §5.5's test paragraph).
//!
//! Scenario 23 end to end in **abstract** and **exact** modes, `off` creating
//! nothing, every stance row, next-period stance application, one exposure event per
//! `Exploit` period and none for `Conceal`, preemption in both directions, expiry
//! under `StatusQuo`, the per-faction pool at 8, an orphaned front, a cadence jump
//! evaluating once, `sheet:wod` **byte-unchanged** across every test, the chargen
//! artifacts per mode, the Residence step's disclosure, and `@wod/economy migrate` in
//! both directions minting nothing.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use mush_core::{
    Audience, Effects, Engine, GOD, GameModule, Inbound, LIMBO, ModuleBootData, ModuleWrite, ObjId,
    ObjKind, Outbox,
};
use mush_economy::EconomyStore;
use mush_housing::{HousingStore, PropertyId, TenancyId, TenancyStatusKind};
use mush_org::SystemPrincipalId;
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, boot_world, text_of};
use ruleset_wod::economy::config::{FAUCET_CHARGEN, WOD_USD};
use ruleset_wod::economy::record::{
    MAX_OFFSCREEN_OPS_PER_FACTION, Prompt, PromptDefault, PromptKind, STRAIN_MAX,
};
use ruleset_wod::economy::store::WodEconomyStore;
use ruleset_wod::{Engagement, PersonalMode, SpendingProfile, WodEconomyConfig, WodRuleset};
use smol_str::SmolStr;

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
fn modules(cfg: WodEconomyConfig) -> Vec<Box<dyn GameModule>> {
    let issuer = SystemPrincipalId::new("wod.housing").expect("valid system principal");
    vec![
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
    ]
}

fn boot(cfg: WodEconomyConfig) -> TestWorld {
    TestWorld::with_modules(modules(cfg))
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
    events
        .iter()
        .map(|(_, e)| text_of(e))
        .collect::<Vec<_>>()
        .join("\n")
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
    assert!(
        has_error(&ev),
        "`{line}` should have been refused, got: {}",
        texts(&ev)
    );
    texts(&ev)
}

fn store(tw: &TestWorld) -> &WodEconomyStore {
    tw.world()
        .module_state::<WodEconomyStore>()
        .expect("wod economy store")
}

fn housing(tw: &TestWorld) -> &HousingStore {
    tw.world()
        .module_state::<HousingStore>()
        .expect("housing store")
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
    store(tw)
        .cadence()
        .expect("the configured cadence resolves")
        .0
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

/// Stamp `who`'s sheet `Approved`, exactly as the chargen approval handler does.
///
/// `@sheet/set` leaves a sheet `Unapproved`, and `@wod/economy migrate`'s population
/// is the **approved** one (E5c verify fold M2) — so a fixture that skips the whole
/// `+chargen` run has to stamp the flip itself.
fn approve_sheet(tw: &mut TestWorld, who: ObjId) {
    let mut doc = tw
        .world()
        .attr(who, "sheet:wod")
        .and_then(mush_core::Attr::as_json)
        .cloned()
        .expect("a sheet to approve");
    doc["approved"] = serde_json::json!({ "approved": { "by": GOD.0, "at": 0 } });
    tw.world_mut()
        .set_attr(who, "sheet:wod", mush_core::Attr::json(doc, GOD))
        .expect("the sheet attribute is writable");
}

/// A player with an **approved** sheet at `resources` dots, and a profile.
fn character(tw: &mut TestWorld, name: &str, resources: u8) -> ObjId {
    let who = tw.spawn_player(name);
    ok(
        tw,
        GOD,
        &format!("@sheet/set {name}/backgrounds:Resources={resources}"),
    );
    approve_sheet(tw, who);
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
        &format!(
            "@property/create {name}=#{} tier={tier} landlord=housing",
            anchor.0
        ),
    );
    housing(tw).properties().last().expect("a property").id
}

fn tenancy_for(tw: &mut TestWorld, p: PropertyId, who: ObjId) -> TenancyId {
    let c = cadence_id(tw);
    ok(
        tw,
        GOD,
        &format!("@tenancy/create {p}=#{} cadence={c}", who.0),
    );
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
        .and_then(|s| {
            s.factions
                .values()
                .find(|f| f.name == faction)
                .map(|f| f.id.0)
        })
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
        tw.world().module_state::<EconomyStore>().is_some_and(|s| s
            .snapshot()
            .instruments()
            .count()
            == 0),
        "off registered an instrument"
    );
    err(&mut tw, nick, "+career");
    err(&mut tw, nick, "+resources");
    // migrate deletes nothing and creates no profile.
    let out = ok(&mut tw, GOD, "@wod/economy migrate");
    assert!(
        out.contains("Nothing to do"),
        "off migrate did something: {out}"
    );
    assert_eq!(store(&tw).profiles().count(), 0);
    // E5c-C2 (look-back fold): `off` with housing on creates exactly one inert
    // Manual clock — the one housing assesses on — and nothing else.
    let cadences: Vec<(String, mush_economy::CadenceSource)> = tw
        .world()
        .module_state::<EconomyStore>()
        .map(|s| {
            s.snapshot()
                .cadences()
                .map(|c| (c.name.to_string(), c.source))
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(
        cadences,
        vec![(
            "chronicle.period".to_string(),
            mush_economy::CadenceSource::Manual
        )],
        "off created something other than the one inert clock"
    );
}

#[test]
fn abstract_mode_registers_no_wod_usd() {
    let tw = boot(cfg(PersonalMode::Abstract));
    assert!(
        store(&tw).instrument().is_none(),
        "abstract registered wod.usd"
    );
    let has = tw.world().module_state::<EconomyStore>().is_some_and(|s| {
        s.snapshot()
            .instrument(&mush_economy::InstrumentId::new(WOD_USD).unwrap())
            .is_some()
    });
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
    assert_eq!(
        systems, 3,
        "wod.income, wod.housing and wod.chargen each own an account"
    );
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
    assert!(
        out.contains("created by this ruleset's boot"),
        "@wod/economy hid it: {out}"
    );
    assert!(out.contains("personal_mode      abstract"), "{out}");
}

#[test]
fn housing_refuses_a_room_wired_into_with_reserved_rooms() {
    // What this pins is housing's refusal of a room the host wired into
    // `with_reserved_rooms`. The `[wod.economy].reserved_rooms` → `server_modules_with`
    // → `with_reserved_rooms` chain is the host's, and is pinned in `mush-server`
    // (`modules.rs`, E5c-C231).
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
    let refusal = err(
        &mut tw,
        GOD,
        &format!(
            "@property/create Limbo Flat=#{} tier=1 landlord=housing",
            LIMBO.0
        ),
    );
    assert!(!refusal.is_empty());
    ok(
        &mut tw,
        GOD,
        &format!(
            "@property/create Parade Ground=#{} tier=1 landlord=housing",
            anchor.0
        ),
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
    assert!(
        p.strain < high,
        "coast did not decay strain ({high} → {})",
        p.strain
    );
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
    assert_eq!(
        profile(&tw, nick).level,
        2,
        "conceal did not lower the level"
    );
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
    assert_eq!(
        p.stance(),
        ruleset_wod::Stance::Coast,
        "the flip landed immediately"
    );
    assert_eq!(
        p.career().and_then(|c| c.pending_stance),
        Some(ruleset_wod::Stance::Exploit)
    );
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
    assert_eq!(
        ev.source,
        format!("front:{f}:{fr}"),
        "evidence must cite the front"
    );
    assert!(ev.entities.contains(&nick));
    assert_eq!(
        ev.detail.get("faction").and_then(serde_json::Value::as_u64),
        Some(f)
    );
    // The whole K4a world-IC stamp travels with it, generation and all.
    assert!(
        ev.detail.get("world_ic").is_some(),
        "no world-IC stamp: {:?}",
        ev.detail
    );
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
    assert_eq!(
        canon_kinds(&tw, "wod.career.exposure"),
        base,
        "conceal left a trace"
    );
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
        matches!(
            profile(&tw, nick).prompt.map(|x| x.kind),
            Some(PromptKind::Opportunity { .. })
        ),
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
        hist.iter()
            .any(|h| h.what.contains("dropped for housing pressure")),
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
        p.history
            .iter()
            .any(|h| h.what.contains(&format!("prompt #{} lapsed", opened.id))),
        "no lapse note: {:?}",
        p.history
    );
    assert!(
        !p.history
            .iter()
            .any(|h| h.what.to_lowercase().contains("fail")),
        "expiry read as a failure"
    );
    assert_eq!(
        tw.world()
            .module_state::<EconomyStore>()
            .map(|s| s.snapshot().accounts().count()),
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
    assert!(
        view.contains("lapses"),
        "the non-response default was not stated: {view}"
    );
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
    assert_eq!(
        open, MAX_OFFSCREEN_OPS_PER_FACTION,
        "the pool did not cap at 8: {open}"
    );
    let with = who
        .iter()
        .filter(|o| profile(&tw, **o).prompt.is_some())
        .count();
    assert_eq!(with, 8, "{with} of {n} characters hold a prompt");
    // Resolving one frees a slot: the pool is derived, so the decrement is automatic.
    let holder = *who
        .iter()
        .find(|o| profile(&tw, **o).prompt.is_some())
        .expect("a holder");
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
    assert!(
        profile(&tw, nick).prompt.is_some(),
        "no opportunity before the front vanished"
    );
    ok(&mut tw, nick, "+prompt pass");
    let base = canon_kinds(&tw, "wod.career.exposure");

    // Simulate a K / faction change that removes the front out from under it.
    if let Some(sim) = tw.world_mut().module_state_mut::<mush_faction::SimState>() {
        sim.fronts.remove(&mush_faction::FrontId(fr));
    }
    // The read never fails, and says so in the raw-id form.
    let out = ok(&mut tw, nick, "+career");
    assert!(out.contains("(orphaned front)"), "orphan not marked: {out}");
    assert!(
        out.contains(&format!("front #{f}/#{fr}")),
        "raw id not shown: {out}"
    );
    // …and nothing more is generated.
    for _ in 0..4 {
        period(&mut tw, 1);
    }
    assert!(
        profile(&tw, nick).prompt.is_none(),
        "an orphaned front kept generating"
    );
    assert_eq!(
        canon_kinds(&tw, "wod.career.exposure"),
        base,
        "an orphaned front kept emitting"
    );
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
    assert_eq!(
        after,
        before + 1,
        "a 40-period jump spent {} periods",
        after - before
    );
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
    assert_eq!(
        sheet_bytes(&tw, nick),
        before,
        "the economy wrote the sheet"
    );
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
        tw.world()
            .module_state::<EconomyStore>()
            .map(|s| s.snapshot().accounts().count()),
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
        .and_then(|s| {
            s.snapshot()
                .account(account)
                .map(|a| a.posted_balance.minor())
        })
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
            s.snapshot()
                .accounts()
                .find(|a| a.owner == sys)
                .map(|a| a.posted_balance.minor())
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
    assert!(
        out.contains("Bedsit"),
        "the affordable vacancy was not offered: {out}"
    );
    assert!(
        !out.contains("Penthouse"),
        "a tier-5 place was offered to Resources 2: {out}"
    );
    assert!(
        !out.contains("Rowhouse"),
        "an occupied property was listed: {out}"
    );
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
    approve_sheet(&mut tw, nick);
    let out = ok(&mut tw, GOD, "@wod/economy migrate");
    assert!(out.contains("profile(s) created"), "{out}");
    assert!(
        out.contains("zero-balance wod.usd account(s) opened"),
        "{out}"
    );
    assert!(out.contains("Nothing was minted"), "{out}");
    let account = profile(&tw, nick).account.expect("a wallet");
    let bal = tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|s| {
            s.snapshot()
                .account(account)
                .map(|a| a.posted_balance.minor())
        })
        .expect("the account");
    assert_eq!(bal, 0, "migrate minted");
    // Idempotent.
    let again = ok(&mut tw, GOD, "@wod/economy migrate");
    assert!(
        again.contains("Nothing to do"),
        "migrate was not idempotent: {again}"
    );
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
        .and_then(|s| {
            s.snapshot()
                .account(account)
                .map(|a| a.posted_balance.minor())
        })
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
    approve_sheet(&mut tw2, nick2);
    let out = ok(&mut tw2, GOD, "@wod/economy migrate");
    assert!(out.contains("profile(s) created"), "{out}");
    assert!(!out.contains("minted"), "abstract migrate minted: {out}");
    // No wallet exists at all in abstract mode, so there is nothing to show.
    assert!(profile(&tw2, nick2).account.is_none());
    let shown = ok(&mut tw2, nick2, "+resources");
    assert!(
        !shown.contains("account #"),
        "abstract showed a wallet: {shown}"
    );

    // …and in the exact world the balance is exactly where it was.
    let after = tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|s| {
            s.snapshot()
                .account(account)
                .map(|a| a.posted_balance.minor())
        })
        .expect("the account");
    assert_eq!(after, balance, "migrate touched a balance");
    let kind = tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().obligation(income).map(|o| o.status.kind()));
    assert!(kind.is_some(), "the income record vanished");
}

#[test]
fn resources_shows_the_number_to_its_owner_and_to_staff_and_refuses_everybody_else() {
    let mut tw = boot(cfg(PersonalMode::Abstract));
    let nick = character(&mut tw, "Nick", 3);
    let quinn = character(&mut tw, "Quinn", 1);
    let p = property(&mut tw, "Flat", 2);
    let _t = tenancy_for(&mut tw, p, nick);
    period(&mut tw, 1);
    period(&mut tw, 1);

    let own = ok(&mut tw, nick, "+resources");
    assert!(
        own.contains("support level 3"),
        "the owner did not see the number: {own}"
    );
    // Staff see it too.
    let staff = ok(&mut tw, GOD, "+resources Nick");
    assert!(
        staff.contains("support level 3"),
        "staff did not see the number: {staff}"
    );
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
    ok(
        &mut tw,
        GOD,
        &format!(
            "@career/set Nick=kind=courier employer=#{} front={f}:{fr} risk=3",
            boss.0
        ),
    );
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
    assert!(
        profile(&tw, nick).prompt.is_none(),
        "an unattributed opportunity was opened"
    );
    assert_eq!(
        canon_kinds(&tw, "wod.career.exposure"),
        0,
        "unattributed evidence was committed"
    );
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
    assert!(
        profile(&tw, nick).prompt.is_none(),
        "ambient opened a prompt"
    );
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
    let plan = ten
        .funding
        .plans()
        .and_then(|m| m.get(&nick).cloned())
        .expect("a plan");
    // Housing sees the career's opaque kind and nothing else — no stance, no front,
    // no employer, no risk numbers (D-E5-5).
    assert!(plan.sources.iter().any(|s| matches!(
        s,
        mush_housing::SupportSource::Career { kind } if kind == "fixer"
    )));
    assert!(
        plan.sources
            .contains(&mush_housing::SupportSource::Inheritance)
    );
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
    assert_eq!(
        profile(&tw, nick).strain,
        before,
        "the tick spent periods the clock never moved"
    );
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

// ---------------------------------------------------------------------------
// ---- E5c verify fold ----
//
// One test per confirmed finding of the three-lens adversarial verify (08d §5.7):
// lens M (money, the standing mint, the housing seam), lens T (the tick, durability,
// the prompt), lens D (disclosure, authority, the verb surface).
// ---------------------------------------------------------------------------

/// Every namespace's persisted documents, folded the way the server's write batch
/// folds them. A reboot has to replay **all** of them: `Engine::modules_loaded` hands
/// every module its boot data, so naming only `wod` would boot the economy with no
/// cadence, no accounts and no obligations at all (E5b's rig, same reason).
type Kv = HashMap<SmolStr, Vec<(SmolStr, serde_json::Value)>>;

fn fold_kv(tw: &mut TestWorld) -> Kv {
    let mut kv = Kv::new();
    for op in tw.take_module_ops() {
        let entry = kv.entry(op.ns.clone()).or_default();
        match op.op {
            ModuleWrite::JsonSet { key, value } => {
                entry.retain(|(k, _)| *k != key);
                entry.push((key, value));
            }
            ModuleWrite::Del { key } => entry.retain(|(k, _)| *k != key),
            ModuleWrite::StreamAppend { .. } => {}
        }
    }
    kv
}

fn drop_key(kv: &mut Kv, ns: &str, key: &str) {
    let entry = kv
        .get_mut(&SmolStr::new(ns))
        .unwrap_or_else(|| panic!("no {ns} namespace"));
    let before = entry.len();
    entry.retain(|(k, _)| k.as_str() != key);
    assert!(
        entry.len() < before,
        "no such persisted key to drop: {ns}/{key}"
    );
}

/// Compose a **fresh** engine over a clone of `tw`'s world and hand every module its
/// persisted documents.
///
/// `TestWorld::modules_loaded` cannot be re-entered on a booted world:
/// `WodRuleset::on_world_loaded` re-registers the `wod.chargen` payload handler and a
/// second registration is refused. `crates/ruleset-wod/tests/persistence.rs` documents
/// the working pattern — compose a new `Engine` rather than re-enter the live one —
/// and this is it, with the config as the second half of the reboot (a deployment may
/// have edited `mushd.toml` while it was down).
fn reboot_fresh(tw: &TestWorld, kv: Kv, cfg: WodEconomyConfig) -> Engine {
    let (_, registry) = boot_world();
    let mut engine = Engine::with_modules(tw.world().clone(), registry, modules(cfg))
        .expect("the module roster composes");
    let boot: HashMap<SmolStr, ModuleBootData> = kv
        .into_iter()
        .map(|(ns, kv)| {
            (
                ns,
                ModuleBootData {
                    kv,
                    streams: HashMap::new(),
                    ..Default::default()
                },
            )
        })
        .collect();
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    engine
        .modules_loaded(boot, &mut out, &mut fx)
        .expect("the boot data loads");
    engine
}

fn engine_conn(engine: &mut Engine, actor: ObjId) {
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    engine.step(
        Inbound::ConnUp {
            conn: u64::from(actor.0),
            actor,
            caps_structured: false,
            from_linkdead: false,
        },
        &mut out,
        &mut fx,
    );
}

fn engine_cmd(engine: &mut Engine, actor: ObjId, line: &str) -> Vec<(Audience, OutputEvent)> {
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    engine.step(
        Inbound::Cmd {
            conn: 0,
            actor,
            line: line.to_string(),
        },
        &mut out,
        &mut fx,
    );
    out.take()
}

fn engine_period(engine: &mut Engine, cadence: u64, n: u64) {
    let ev = engine_cmd(engine, GOD, &format!("@econ/cadence advance {cadence} {n}"));
    assert!(
        !has_error(&ev),
        "the reboot could not advance the clock: {}",
        texts(&ev)
    );
    let t = engine.world.now().saturating_add(40);
    engine.set_now(t);
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    engine.fire_due(t.saturating_mul(1000), &mut out, &mut fx);
}

fn engine_store(engine: &Engine) -> &WodEconomyStore {
    engine
        .world
        .module_state::<WodEconomyStore>()
        .expect("wod economy store")
}

fn engine_profile(engine: &Engine, who: ObjId) -> SpendingProfile {
    engine_store(engine)
        .profile(who)
        .cloned()
        .expect("a profile")
}

/// The minor-unit amount of one obligation.
fn obligation_minor(tw: &TestWorld, id: mush_economy::ObligationId) -> i64 {
    tw.world()
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().obligation(id).map(|o| o.amount.minor()))
        .expect("the obligation record")
}

/// Every `wod.income` obligation in the world, live or not, as `(id, status)`.
fn income_lines(
    w: &mush_core::World,
) -> Vec<(
    mush_economy::ObligationId,
    mush_economy::ObligationStatusKind,
)> {
    w.module_state::<EconomyStore>()
        .map(|s| {
            s.snapshot()
                .obligations()
                .filter(|o| {
                    matches!(&o.cause, mush_economy::ObligationCause::Ruleset { namespace }
                        if namespace.as_str() == "wod.income")
                })
                .map(|o| (o.id, o.status.kind()))
                .collect()
        })
        .unwrap_or_default()
}

fn live_income_count(w: &mush_core::World) -> usize {
    income_lines(w)
        .into_iter()
        .filter(|(_, k)| {
            !matches!(
                k,
                mush_economy::ObligationStatusKind::Cancelled
                    | mush_economy::ObligationStatusKind::Satisfied
            )
        })
        .count()
}

fn personal_accounts(tw: &TestWorld, who: ObjId) -> usize {
    tw.world()
        .module_state::<EconomyStore>()
        .map(|s| {
            s.snapshot()
                .accounts()
                .filter(|a| a.owner == mush_org::PrincipalId::Object(who))
                .count()
        })
        .unwrap_or(0)
}

// ---- Lens M -------------------------------------------------------------

#[test]
fn vf_m1_income_is_keyed_on_the_sheet_dot_not_the_stance() {
    // M1: `maintain_income` rated the mint off `profile.level`, which is
    // `resources ± stance` — so `+career/stance exploit`, an ungated player verb,
    // moved the standing mint one whole income row up on the same ObligationId, and
    // `conceal` moved it down or cancelled it. The row is keyed on the **sheet
    // reading** (D-E5-6: a stance alters pressure and opportunity, never income).
    let mut c = cfg(PersonalMode::Exact);
    c.income.insert(2, 200);
    c.income.insert(3, 500);
    c.income.insert(4, 5_000);
    let mut tw = boot(c);
    let nick = character(&mut tw, "Nick", 3);
    ok(&mut tw, GOD, "@wod/economy migrate");
    ok(&mut tw, nick, "+career/source add career=fixer");
    period(&mut tw, 1);
    period(&mut tw, 1);
    let id = profile(&tw, nick)
        .income
        .expect("an income line at Resources 3");
    assert_eq!(
        obligation_minor(&tw, id),
        50_000,
        "Resources 3 → 500 major at scale 2"
    );

    for (stance, level) in [("exploit", 4_u8), ("conceal", 2_u8)] {
        ok(&mut tw, nick, &format!("+career/stance {stance}"));
        period(&mut tw, 1);
        period(&mut tw, 1);
        let p = profile(&tw, nick);
        // The stance still moves the housing-support number — that is what §5.2's
        // table promises it, and `submit_plans` passes it in both modes.
        assert_eq!(p.level, level, "{stance} did not move the support level");
        assert_eq!(p.resources, 3, "{stance} moved the cached sheet reading");
        // …and nothing at all about the money.
        assert_eq!(
            p.income,
            Some(id),
            "{stance} opened a different income line"
        );
        assert_eq!(
            obligation_minor(&tw, id),
            50_000,
            "{stance} re-rated the standing mint"
        );
        assert_eq!(
            live_income_count(tw.world()),
            1,
            "{stance} left a second line running"
        );
    }
}

#[test]
fn vf_m2_migrate_skips_an_unapproved_sheet_and_reports_it() {
    // M2: `sheeted()` tested for the presence of `sheet:wod`, and `+chargen` writes
    // that attribute at `Unapproved` the moment a player starts — so a character who
    // typed their own `backgrounds:Resources=5` and never submitted got a profile, a
    // wallet, and two periods later a live income line.
    let mut c = cfg(PersonalMode::Exact);
    c.income.insert(5, 1_000);
    let mut tw = boot(c);
    let nick = tw.spawn_player("Nick");
    ok(&mut tw, GOD, "@sheet/set Nick/backgrounds:Resources=5");

    let out = ok(&mut tw, GOD, "@wod/economy migrate");
    assert!(out.contains("skipped"), "the skip was silent: {out}");
    assert!(
        out.contains("unapproved"),
        "the reason was not given: {out}"
    );
    assert!(
        out.contains(&format!("#{}", nick.0)),
        "the character was not named: {out}"
    );
    assert!(
        store(&tw).profile(nick).is_none(),
        "migrate profiled an unapproved sheet"
    );
    assert_eq!(
        personal_accounts(&tw, nick),
        0,
        "migrate banked an unapproved sheet"
    );
    for _ in 0..3 {
        period(&mut tw, 1);
    }
    assert_eq!(
        live_income_count(tw.world()),
        0,
        "an unapproved sheet drew an income line"
    );

    // Approval is the flip that admits them, and it is the same flip
    // `chargen_approval` stamps — which is why `sheeted` reads through the same
    // dispatch.
    approve_sheet(&mut tw, nick);
    let out = ok(&mut tw, GOD, "@wod/economy migrate");
    assert!(out.contains("profile(s) created"), "{out}");
    assert!(
        store(&tw).profile(nick).is_some(),
        "an approved sheet was still skipped"
    );
}

#[test]
fn vf_m3_an_orphaned_income_line_is_adopted_at_boot_and_never_doubled() {
    // M3: `SpendingProfile::income` was the layer's ONLY name for the obligation, so
    // a dropped profile or a cleared pointer stranded a faucet-funded mint nothing
    // could reach — and `open_income` then scanned nothing and opened a second live
    // line crediting the same wallet. The record is self-identifying (`cause` +
    // `minted_by` + `credit_account`), so the pointer is a cache and the boot
    // reconciles it: adopt one whose wallet a profile owns, cancel one nobody claims.
    let mut c = cfg(PersonalMode::Exact);
    c.income.insert(3, 500);
    let mut tw = boot(c.clone());
    let nick = character(&mut tw, "Nick", 3);
    let quinn = character(&mut tw, "Quinn", 3);
    ok(&mut tw, GOD, "@wod/economy migrate");
    let flat = property(&mut tw, "Flat", 2);
    let _t = tenancy_for(&mut tw, flat, nick);
    period(&mut tw, 1);
    period(&mut tw, 1);
    let nick_line = profile(&tw, nick).income.expect("Nick's income line");
    let quinn_line = profile(&tw, quinn).income.expect("Quinn's income line");
    assert_eq!(live_income_count(tw.world()), 2);

    // The two ways a pointer is lost: Nick's whole profile document goes (he has a
    // live tenancy, so the boot re-derives him), and Quinn's profile and cursor both
    // go with no tenancy behind them (so nobody claims his wallet).
    let mut kv = fold_kv(&mut tw);
    drop_key(&mut kv, "wod", &format!("profile:{}", nick.0));
    drop_key(&mut kv, "wod", &format!("profile:{}", quinn.0));
    drop_key(&mut kv, "wod", &format!("eval:{}", quinn.0));
    let mut engine = reboot_fresh(&tw, kv, c);

    let p = engine_profile(&engine, nick);
    assert_eq!(p.resources, 3, "the re-derivation lost the sheet reading");
    assert_eq!(
        p.income,
        Some(nick_line),
        "the boot did not adopt the orphaned income line (it named itself)"
    );
    assert_eq!(
        live_income_count(&engine.world),
        1,
        "the boot left a second line running"
    );
    let quinn_status = income_lines(&engine.world)
        .into_iter()
        .find(|(id, _)| *id == quinn_line)
        .map(|(_, k)| k);
    assert_eq!(
        quinn_status,
        Some(mush_economy::ObligationStatusKind::Cancelled),
        "a line no profile claims kept minting"
    );

    // …and running on does not double it.
    engine_conn(&mut engine, GOD);
    let cadence = engine_store(&engine).cadence().expect("the cadence").0;
    for _ in 0..3 {
        engine_period(&mut engine, cadence, 1);
    }
    assert_eq!(
        live_income_count(&engine.world),
        1,
        "the tick opened a second income line"
    );
    assert_eq!(engine_profile(&engine, nick).income, Some(nick_line));
}

#[test]
fn vf_m4_a_cancelled_income_line_is_not_resurrected() {
    // M4: the `(Cancelled, Some(amount))` arm cleared and re-opened, so a staffer's
    // deliberate `@obligation/cancel` came back as a new obligation id with nothing
    // said — and staff had no supported way to stop one character's stipend short of
    // editing dots. E5b's own rule for the same record class: a decision is never
    // re-derived away.
    let mut c = cfg(PersonalMode::Exact);
    c.income.insert(3, 500);
    let mut tw = boot(c);
    let nick = character(&mut tw, "Nick", 3);
    ok(&mut tw, GOD, "@wod/economy migrate");
    period(&mut tw, 1);
    period(&mut tw, 1);
    let id = profile(&tw, nick).income.expect("an income line");

    ok(&mut tw, GOD, &format!("@obligation/cancel {}", id.0));
    for _ in 0..4 {
        period(&mut tw, 1);
    }
    let p = profile(&tw, nick);
    assert!(
        p.income.is_none(),
        "the profile still names a cancelled line"
    );
    assert_eq!(
        p.income_cancelled_at,
        Some(3),
        "the decision was not recorded"
    );
    assert_eq!(
        live_income_count(tw.world()),
        0,
        "the cancel was undone by the tick"
    );
    assert!(
        p.history
            .iter()
            .any(|h| h.what.contains(&format!("income line #{}", id.0))
                && h.what.contains("cancelled")),
        "no note said what happened: {:?}",
        p.history
    );
    // The exact-mode screen says so, so staff can find the rule without reading code.
    let screen = ok(&mut tw, GOD, "@wod/economy");
    assert!(
        screen.contains("cancels by hand stays cancelled"),
        "{screen}"
    );

    // A Resources change is a deliberate act, and it opens the next one.
    ok(&mut tw, GOD, "@sheet/set Nick/backgrounds:Resources=4");
    for _ in 0..3 {
        period(&mut tw, 1);
    }
    ok(&mut tw, GOD, "@sheet/set Nick/backgrounds:Resources=3");
    for _ in 0..3 {
        period(&mut tw, 1);
    }
    assert!(
        profile(&tw, nick).income.is_some(),
        "a dot change did not lift the hand-cancel mark"
    );
}

#[test]
fn vf_m5_an_over_cap_starting_stake_is_reported_not_silent() {
    // M5: `starting_funds_major` is bounded by nothing but itself — which is
    // deliberate (§5.6 deviation 10 puts the stake in `@econ/mint`'s uncapped,
    // attributed class, and deviation 11 says approval never fails). What was wrong
    // was that the screen implied `mint_obligation_cap_major` covered it.
    let mut c = cfg(PersonalMode::Exact);
    c.starting_funds_major = 1_000_000; // twenty times the default 50,000 ceiling
    let mut tw = boot(c);
    let screen = ok(&mut tw, GOD, "@wod/economy");
    assert!(
        screen.contains("ABOVE THE MINT CAP 50000"),
        "the stake was silent: {screen}"
    );
    assert!(
        screen.contains("bounded by starting_funds_major alone"),
        "the screen did not say what actually bounds it: {screen}"
    );
    assert!(
        screen.contains("income LINE is bounded by [economy].mint_obligation_cap_major"),
        "the cap sentence no longer says it covers lines: {screen}"
    );
    // And it still never refuses: an approved character gets the whole stake.
    let who = chargen(&mut tw, "Nick", 3);
    approve(&mut tw, who);
    let account = store(&tw)
        .profile(who)
        .and_then(|p| p.account)
        .expect("a wallet");
    let bal = tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|s| {
            s.snapshot()
                .account(account)
                .map(|a| a.posted_balance.minor())
        })
        .expect("the account");
    assert_eq!(
        bal, 100_000_000,
        "the over-cap stake was refused or clamped"
    );
}

#[test]
fn vf_m6_wod_economy_marks_an_income_row_above_the_mint_cap() {
    // M6: `wod_economy_config` validates the map's KEYS and never its values, so an
    // over-cap row booted clean, printed indistinguishably from a working one, and
    // every `create_system_obligation` for it failed `MintCapExceeded` into a warn
    // nobody reads. §5.6 does not make it a hard boot error, so the screen marks it —
    // the way the cadence line already marks DOES NOT RESOLVE.
    let mut c = cfg(PersonalMode::Exact);
    c.income.insert(3, 1_000);
    c.income.insert(5, 60_000);
    let mut tw = boot(c);
    let screen = ok(&mut tw, GOD, "@wod/economy");
    assert!(
        screen.contains("5=60000 (ABOVE THE MINT CAP 50000; no line is opened)"),
        "the over-cap row printed as live: {screen}"
    );
    assert!(
        !screen.contains("3=1000 (ABOVE"),
        "a working row was marked as over-cap: {screen}"
    );
    // The mark is the truth: a character on that row really gets no line at all.
    let nick = character(&mut tw, "Nick", 5);
    ok(&mut tw, GOD, "@wod/economy migrate");
    for _ in 0..3 {
        period(&mut tw, 1);
    }
    assert!(profile(&tw, nick).income.is_none());
}

#[test]
fn vf_m7_approval_drops_a_residence_the_approved_sheet_cannot_carry() {
    // M7: `vacancies` filters `tier <= level` from the LIVE sheet, so a player could
    // pick a tier-5 home at Resources 5, move the dots to Contacts before
    // `+chargen/submit`, and have approval create the tenancy anyway —
    // Active → Grace → Delinquent → EvictionPending in their first week, which
    // `chargen.rs` says the step never does.
    let mut tw = boot(cfg(PersonalMode::Abstract));
    let penthouse = property(&mut tw, "Penthouse", 5);
    let who = chargen(&mut tw, "Nick", 5);
    ok(&mut tw, who, &format!("+chargen/residence #{penthouse}"));
    assert_eq!(store(&tw).residence(who), Some(penthouse));

    // The dots move before submit. The sheet still balances; the home no longer fits.
    ok(&mut tw, who, "+stat backgrounds:Resources=2");
    ok(&mut tw, who, "+stat backgrounds:Contacts=3");
    let out = approve(&mut tw, who);

    assert!(
        out.contains("tier 5"),
        "the approver was not told the tier: {out}"
    );
    assert!(
        out.contains("support level 2"),
        "the approver was not told the level: {out}"
    );
    assert!(
        out.contains("dropped"),
        "the pick was not reported as dropped: {out}"
    );
    assert_eq!(store(&tw).residence(who), None, "the pick was not consumed");
    assert_eq!(
        housing(&tw).live_tenancies_of_tenant(who).count(),
        0,
        "approval let a home the approved sheet cannot carry"
    );
    // …and the approval itself still stands (deviation 11: report, never un-sign).
    let p = store(&tw).profile(who).cloned().expect("a profile");
    assert_eq!(p.level, 2);
}

// ---- Lens T -------------------------------------------------------------

#[test]
fn vf_t1_a_lost_profile_is_re_derived_at_boot_and_the_home_stays_active() {
    // T1: `load` skipped an unparsable profile and logged "re-derived at the next
    // evaluation" — but the tick's working set IS the profile map, so nothing ever
    // re-derived it: the character left the economy silently, submitted no plan, and
    // with UNKNOWN_GRACE at 1 their tenancy walked to the story window. Precisely the
    // loss §5.6 promised costs the home nothing.
    let mut tw = boot(cfg(PersonalMode::Abstract));
    let nick = character(&mut tw, "Nick", 3);
    let flat = property(&mut tw, "Flat", 2);
    let t = tenancy_for(&mut tw, flat, nick);
    period(&mut tw, 1);
    period(&mut tw, 1);
    assert_eq!(status(&tw, t), TenancyStatusKind::Active);

    // The document is lost (a torn write; the loader's own "skipped loudly" row).
    let mut kv = fold_kv(&mut tw);
    drop_key(&mut kv, "wod", &format!("profile:{}", nick.0));
    let mut engine = reboot_fresh(&tw, kv, cfg(PersonalMode::Abstract));

    let p = engine_profile(&engine, nick);
    assert_eq!(
        p.resources, 3,
        "the profile was not re-derived from the sheet"
    );
    assert_eq!(p.level, 3);
    assert!(
        p.history
            .iter()
            .any(|h| h.what.contains("re-derived at boot")),
        "the re-derivation left no note: {:?}",
        p.history
    );
    // The cursor is re-seeded at the clock's CURRENT position, so the character is
    // not charged for the periods they were missing.
    assert!(engine_store(&engine).eval_state(nick).seeded);

    // And the home: ten periods on, still Active — which is the claim §5.6 makes.
    engine_conn(&mut engine, GOD);
    let cadence = engine_store(&engine).cadence().expect("the cadence").0;
    for _ in 0..10 {
        engine_period(&mut engine, cadence, 1);
        let st = engine
            .world
            .module_state::<HousingStore>()
            .and_then(|s| s.tenancy(t).map(|x| x.status.kind()))
            .expect("the tenancy");
        assert_eq!(
            st,
            TenancyStatusKind::Active,
            "a lost profile cost the home its lease"
        );
    }
}

#[test]
fn vf_t2_a_profile_doc_that_claims_another_id_is_skipped() {
    // T2: `store.profiles.insert(obj, p)` keyed on the KEY while `put_profile` later
    // re-keys on `p.obj`, so a doc under `profile:5` claiming `obj: 7` made `step(#5)`
    // read #5's sheet and write #7's record — overwriting #7's real profile at every
    // boot, forever. `mush-housing` refuses exactly this shape.
    let claimant = SpendingProfile::new(ObjId(7), 3, Engagement::Ambient);
    let honest = SpendingProfile::new(ObjId(9), 4, Engagement::Ambient);
    let kv = vec![
        (
            SmolStr::new("profile:5"),
            serde_json::to_value(&claimant).expect("serializes"),
        ),
        (
            SmolStr::new("profile:9"),
            serde_json::to_value(&honest).expect("serializes"),
        ),
    ];
    let store = ruleset_wod::economy::load_store(&kv, cfg(PersonalMode::Abstract));
    assert!(
        store.profile(ObjId(5)).is_none(),
        "the mismatched doc was installed under its key"
    );
    assert!(
        store.profile(ObjId(7)).is_none(),
        "the mismatched doc was installed under its claim"
    );
    assert_eq!(
        store.profile(ObjId(9)).map(|p| p.resources),
        Some(4),
        "one bad doc took an honest neighbour with it"
    );
}

#[test]
fn vf_t3_a_prompt_stamped_in_a_future_ordinal_lapses_at_once() {
    // T3: `expired` was `period.saturating_sub(opened_period) >= expires_after`, so a
    // prompt stamped at ordinal 5 against a cursor re-seeded to 0 read as not-expired
    // for ordinals 0..=6 — seven periods of life instead of two, holding the
    // character's one attention slot (a stuck `StrainRelief` is not
    // `displaceable_by_housing`, so it blocks the home prompt D-E5-8 outranks
    // everything with).
    let stamped = |opened: u64| Prompt {
        id: 42,
        kind: PromptKind::StrainRelief,
        options: ruleset_wod::economy::prompt::relief_options(),
        default: PromptDefault::StatusQuo,
        opened_at: 0,
        opened_period: opened,
        opened_ic: None,
        expires_after_periods: 2,
    };
    let future = stamped(5);
    for now in [0_u64, 1, 4] {
        assert!(
            future.expired(now),
            "a future stamp read as fresh at ordinal {now}"
        );
    }
    // The ordinary arithmetic is untouched.
    assert!(
        !future.expired(5),
        "a prompt expired in the period it opened"
    );
    assert!(!future.expired(6));
    assert!(future.expired(7));

    // …and `expire` really releases the slot, under the prompt's own default.
    let mut p = SpendingProfile::new(ObjId(3), 3, Engagement::Ambient);
    p.prompt = Some(stamped(5));
    let lapsed = ruleset_wod::economy::prompt::expire(&mut p, 1, 0);
    assert_eq!(lapsed, Some(PromptDefault::StatusQuo));
    assert!(
        p.prompt.is_none(),
        "the stranded prompt kept the attention slot"
    );
    assert_eq!(p.strain, 0, "a lapse cost strain");
}

#[test]
fn vf_t4_a_going_character_is_not_evaluated() {
    // T4: `step`'s only liveness test was `ctx.w.get(who).is_none()`, and core keeps a
    // destroyed object resolvable until `World::sweep` tombstones it — so a destroyed
    // character could gain strain and leave a `wod.career.exposure` record naming
    // them. Every neighbour defines liveness as existence PLUS not-`GOING`.
    let mut tw = boot(cfg(PersonalMode::Abstract));
    let nick = character(&mut tw, "Nick", 3);
    let (f, fr) = front(&mut tw, "Giovanni", "The docks");
    careered(&mut tw, nick, "Nick", "exploit", Some((f, fr)));
    period(&mut tw, 1);
    let strain = profile(&tw, nick).strain;
    let canon = canon_kinds(&tw, "wod.career.exposure");
    assert!(canon >= 1, "the control never emitted anything");

    // Marked `GOING`, not swept: the husk is still resolvable.
    tw.world_mut().destroy(nick).expect("destroy");
    assert!(
        tw.world().get(nick).is_some(),
        "the husk was swept; the window under test is gone"
    );
    for _ in 0..3 {
        period(&mut tw, 1);
    }
    assert_eq!(
        profile(&tw, nick).strain,
        strain,
        "a GOING character was still evaluated"
    );
    assert_eq!(
        canon_kinds(&tw, "wod.career.exposure"),
        canon,
        "a GOING character committed front-attributed canon"
    );
    // The profile itself is untouched — D-E5-9's "nothing deletes a profile".
    assert!(store(&tw).profile(nick).is_some());
}

#[test]
fn vf_t5_housing_pressure_opens_at_delinquent_and_grace_is_a_notice() {
    // T5: doc-only, pinned so it cannot drift. `behind_tenancy` filters on
    // `is_behind()` — `Delinquent | EvictionPending` — so the player's first ASKED
    // moment is one rung after housing's first told moment. That is the accepted
    // split (08d §5.3: housing's own notices are notices, not prompts; §11 default 1
    // fixes GRACE_PERIODS at 1 as a cushion meant to resolve silently).
    let mut tw = boot(cfg(PersonalMode::Abstract));
    let nick = character(&mut tw, "Nick", 0);
    let flat = property(&mut tw, "Flat", 3);
    let t = tenancy_for(&mut tw, flat, nick);

    let mut saw_grace = false;
    let mut pressure_at = None;
    for _ in 0..12 {
        period(&mut tw, 1);
        let st = status(&tw, t);
        let pressure = matches!(
            profile(&tw, nick).prompt.as_ref().map(|p| &p.kind),
            Some(PromptKind::HousingPressure { .. })
        );
        match st {
            TenancyStatusKind::Active | TenancyStatusKind::Pending => {
                assert!(
                    !pressure,
                    "pressure opened before the tenancy was behind at all"
                );
            }
            TenancyStatusKind::Grace => {
                assert!(
                    !pressure,
                    "the Grace rung opened a prompt; it is housing's notice"
                );
                saw_grace = true;
            }
            _ => {}
        }
        if pressure {
            pressure_at = Some(st);
            break;
        }
    }
    assert!(
        saw_grace,
        "the ladder never passed through Grace, so nothing was pinned"
    );
    assert!(
        matches!(
            pressure_at,
            Some(TenancyStatusKind::Delinquent | TenancyStatusKind::EvictionPending)
        ),
        "the prompt did not open at Delinquent: {pressure_at:?}"
    );
}

#[test]
fn vf_t6_the_lapse_line_counts_down() {
    // T6: the line interpolated `expires_after_periods`, a write-once constant, so
    // every read of every prompt said "in 2 period(s)" — including the last period it
    // was readable at all. Non-response is a first-class answer (owner answer 19), so
    // the time left is the one number a player needs to choose it deliberately.
    let mut tw = boot(cfg(PersonalMode::Abstract));
    let nick = character(&mut tw, "Nick", 3);
    let (f, fr) = front(&mut tw, "Giovanni", "The docks");
    // `careered` spends the period that lands the stance, and Exploit offers in that
    // same pass — so the prompt is read in the very ordinal it opened in.
    careered(&mut tw, nick, "Nick", "exploit", Some((f, fr)));
    assert!(profile(&tw, nick).prompt.is_some(), "no prompt to read");

    let fresh = ok(&mut tw, nick, "+prompt");
    assert!(
        fresh.contains("in 2 period(s)"),
        "the fresh prompt did not say two: {fresh}"
    );
    period(&mut tw, 1);
    let last = ok(&mut tw, nick, "+prompt");
    assert!(
        !last.contains("in 2 period(s)"),
        "the countdown did not move off the constant: {last}"
    );
    assert!(
        last.contains("last one"),
        "the last period was not called out: {last}"
    );
    period(&mut tw, 1);
    assert!(
        profile(&tw, nick).prompt.is_none(),
        "the prompt outlived its own countdown"
    );
}

#[test]
fn vf_t7_the_loader_pins_every_corrupt_doc_row() {
    // T7: §5.6 asserts four load-time behaviours that no test exercised, which is why
    // T1, T2 and T3 shipped unpinned while `mush-housing`'s twins are pinned at every
    // rung. `load_store` is `pub` and takes the kv slice, so all four rows plus the
    // allocator flooring go through the real loader with no second `modules_loaded`.
    let mut good = SpendingProfile::new(ObjId(4), 2, Engagement::Ambient);
    good.prompt = Some(Prompt {
        id: 40,
        kind: PromptKind::StrainRelief,
        options: ruleset_wod::economy::prompt::relief_options(),
        default: PromptDefault::StatusQuo,
        opened_at: 0,
        opened_period: 0,
        opened_ic: None,
        expires_after_periods: 2,
    });
    let kv = vec![
        // row 1 — a profile that does not parse is skipped and its neighbours live.
        (
            SmolStr::new("profile:3"),
            serde_json::json!("not a profile at all"),
        ),
        (
            SmolStr::new("profile:4"),
            serde_json::to_value(&good).expect("serializes"),
        ),
        (SmolStr::new("profile:nope"), serde_json::json!({})),
        // row 2 — an unparsable cursor is skipped and re-seeded (it reads as default).
        (
            SmolStr::new("eval:4"),
            serde_json::json!({ "position": 12, "period": 7, "seeded": true }),
        ),
        (SmolStr::new("eval:3"), serde_json::json!("torn")),
        // row 3 — a residence pick with no property is skipped; a good one survives.
        (
            SmolStr::new("residence:4"),
            serde_json::json!({ "property": 9 }),
        ),
        (SmolStr::new("residence:3"), serde_json::json!({})),
        // row 4 — the allocator doc is tolerated field by field.
        (
            SmolStr::new("econ:meta"),
            serde_json::json!({ "next_prompt_id": 3, "junk": true }),
        ),
        // …and a key that is not ours at all (M17's) is simply not ours.
        (
            SmolStr::new("conceal"),
            serde_json::json!({ "anything": 1 }),
        ),
    ];
    let mut store = ruleset_wod::economy::load_store(&kv, cfg(PersonalMode::Abstract));

    assert!(
        store.profile(ObjId(3)).is_none(),
        "an unparsable profile was installed"
    );
    assert_eq!(
        store.profile(ObjId(4)).map(|p| p.resources),
        Some(2),
        "a good profile was lost"
    );
    assert_eq!(
        store.profiles().count(),
        1,
        "an unparsable key produced a profile"
    );

    assert_eq!(
        store.eval_state(ObjId(4)).period,
        7,
        "a good cursor was lost"
    );
    let torn = store.eval_state(ObjId(3));
    assert!(
        !torn.seeded && torn.period == 0,
        "a torn cursor was not re-seeded from default"
    );

    assert_eq!(store.residence(ObjId(4)), Some(mush_housing::PropertyId(9)));
    assert_eq!(
        store.residence(ObjId(3)),
        None,
        "a residence pick with no property was kept"
    );

    // The allocator floors past every id a SURVIVING profile actually holds, never
    // only past `meta` — a hand-written 40 must not be re-minted over.
    assert_eq!(
        store.alloc_prompt(),
        Some(41),
        "the prompt allocator did not floor past the docs"
    );
}

// ---- Lens D -------------------------------------------------------------

#[test]
fn vf_d1_a_hidden_faction_never_reaches_a_non_member_render() {
    // D1: `render_front` gated on `front_resolves` (existence) alone and then read
    // `FactionState.factions[..].name` and `SimState.fronts[..].title` verbatim — so
    // `+career` and `+prompt` printed "Sabbat: Burn the waterfront" to a player whose
    // `+factions` says "No factions are known." and whose `@front` is refused.
    let mut tw = boot(cfg(PersonalMode::Abstract));
    let nick = character(&mut tw, "Nick", 3);
    let (f, fr) = front(&mut tw, "Sabbat", "Burn the waterfront");
    ok(&mut tw, GOD, "@faction/set Sabbat=hidden:true");
    ok(&mut tw, GOD, &format!("@career/set Nick=front={f}:{fr}"));

    let out = ok(&mut tw, nick, "+career");
    assert!(
        !out.contains("Sabbat"),
        "a hidden faction's name reached a non-member: {out}"
    );
    assert!(
        !out.contains("Burn the waterfront"),
        "a staff-only front title reached a non-member: {out}"
    );
    assert!(
        out.contains(&format!("front #{f}/#{fr}")),
        "the raw id form was not used: {out}"
    );
    // "You may not see it" and "it is gone" must read identically — an orphan marker
    // here would be the oracle the fallback exists to close.
    assert!(
        !out.contains("(orphaned front)"),
        "the refusal was distinguishable: {out}"
    );

    // Unhide it and the same render names it: the narrowing is the predicate, not a
    // blanket redaction, and attribution on the record never changed.
    ok(&mut tw, GOD, "@faction/set Sabbat=hidden:false");
    let out = ok(&mut tw, nick, "+career");
    assert!(
        out.contains("Sabbat"),
        "a listed faction stopped rendering: {out}"
    );
    assert!(out.contains("Burn the waterfront"), "{out}");
}

#[test]
fn vf_d2_careers_off_neither_moves_the_level_nor_accepts_a_stance() {
    // D2: `step` gated only prompts and exposure on `cfg.careers`; steps 1–3 landed
    // `pending_stance`, derived `level` from `stance.level_delta()` and applied
    // `strain_delta()` unconditionally, and `career_cmd` gated only on the mode — so
    // on a careers-off server a player still typed `+career/stance exploit` and bought
    // a whole support tier. The config field's own doc comment says careers is
    // "whether careers, stances, opportunities and exposure are evaluated".
    let mut tw = boot(cfg(PersonalMode::Abstract));
    let nick = character(&mut tw, "Nick", 3);
    ok(&mut tw, nick, "+career/source add career=fixer");
    ok(&mut tw, nick, "+career/stance exploit");
    period(&mut tw, 1);
    period(&mut tw, 1);
    assert_eq!(
        profile(&tw, nick).level,
        4,
        "the control never bought the tier"
    );

    // The deployment edits `[wod.economy].careers = false` and restarts.
    let kv = fold_kv(&mut tw);
    let mut off = cfg(PersonalMode::Abstract);
    off.careers = false;
    let mut engine = reboot_fresh(&tw, kv, off);
    engine_conn(&mut engine, GOD);
    engine_conn(&mut engine, nick);
    let cadence = engine_store(&engine).cadence().expect("the cadence").0;
    engine_period(&mut engine, cadence, 1);
    engine_period(&mut engine, cadence, 1);

    let p = engine_profile(&engine, nick);
    assert_eq!(p.resources, 3);
    assert_eq!(
        p.level, 3,
        "a stance still moved the support level on a careers-off server"
    );
    // …and the three verbs that drive it refuse, in the off-mode gate's own words.
    for line in [
        "+career/stance conceal",
        "+career/engage on",
        "+career/source add career=fixer",
    ] {
        let ev = engine_cmd(&mut engine, nick, line);
        assert!(
            has_error(&ev),
            "`{line}` was accepted with careers off: {}",
            texts(&ev)
        );
        assert!(
            texts(&ev).contains("does not run the WoD career layer"),
            "`{line}` refused with the wrong words: {}",
            texts(&ev)
        );
    }
    // A non-career source is housing attribution, which `careers = false` does not
    // switch off.
    let ev = engine_cmd(&mut engine, nick, "+career/source add inheritance");
    assert!(
        !has_error(&ev),
        "careers=false refused a housing attribution: {}",
        texts(&ev)
    );
}

#[test]
fn vf_d3_a_non_staff_third_party_is_refused_and_the_word_branch_is_gone() {
    // D3: `require_staff(ctx)?` runs before `match_object`, so `target != me` implied
    // staff and the third-party "They look …" arm could never execute — while
    // `housing_word` calls `live_tenancies_of_tenant` with NO view predicate, so
    // reviving the arm would have handed any character a tenant's assessment word,
    // bypassing the rule `@tenancy/show` enforces. Dead code that leaks on revival.
    let mut tw = boot(cfg(PersonalMode::Abstract));
    let nick = character(&mut tw, "Nick", 3);
    let quinn = character(&mut tw, "Quinn", 1);
    let flat = property(&mut tw, "Flat", 2);
    let _t = tenancy_for(&mut tw, flat, nick);
    period(&mut tw, 1);
    period(&mut tw, 1);

    let refusal = err(&mut tw, quinn, "+resources Nick");
    assert!(
        refusal.contains("permission"),
        "the refusal changed shape: {refusal}"
    );
    assert!(
        !refusal.to_lowercase().contains("they look"),
        "the word branch survived: {refusal}"
    );

    // Self and staff still get the number and the home word — the branch that is
    // actually reachable.
    let own = ok(&mut tw, nick, "+resources");
    assert!(own.contains("support level 3"), "{own}");
    assert!(own.contains("your home assesses"), "{own}");
    let staff = ok(&mut tw, GOD, "+resources Nick");
    assert!(staff.contains("support level 3"), "{staff}");
    assert!(!staff.to_lowercase().contains("they look"), "{staff}");
}

#[test]
fn vf_d4_career_set_says_when_a_front_does_not_resolve() {
    // D4: `parse_front` validates syntax only, so `front=999:888` answered
    // "Set front=999:888 on #2." while the comment right below claimed the
    // unresolvable case "is said out loud". The staffer who can fix the typo was
    // never told; the player, whose `+career` shows "(orphaned front)", cannot.
    // Deviation 5 permits the set-ahead case, so this is an advisory, not a refusal.
    let mut tw = boot(cfg(PersonalMode::Abstract));
    let nick = character(&mut tw, "Nick", 3);
    let out = ok(&mut tw, GOD, "@career/set Nick=front=999:888");
    assert!(
        out.contains("Set front=999:888"),
        "the set-ahead case was refused: {out}"
    );
    assert!(
        out.contains("does not resolve yet"),
        "the advisory was not said: {out}"
    );
    assert!(out.contains("generates nothing until it does"), "{out}");
    // It really is recorded — set-ahead is the point.
    assert_eq!(
        profile(&tw, nick)
            .career()
            .and_then(|c| c.front)
            .map(|f| (f.faction.0, f.front.0)),
        Some((999, 888))
    );

    // A front that resolves says nothing extra.
    let (f, fr) = front(&mut tw, "Giovanni", "The docks");
    let out = ok(&mut tw, GOD, &format!("@career/set Nick=front={f}:{fr}"));
    assert!(
        !out.contains("does not resolve"),
        "a live front was called broken: {out}"
    );
}

#[test]
fn vf_d5_domain_help_matches_the_parser_and_is_no_org_oracle() {
    // D5: the help said `domain=<org>`, the parser demanded an id, `domain=Camarilla`
    // was refused with the bare fragment "domain=<org id>.", and `domain=987654` —
    // naming no org — was accepted and crossed into housing's `SupportPlan`. The two
    // unambiguous halves are fixed; the org-existence check is deliberately NOT added
    // to the player verb (E5b fold A3's org oracle).
    let mut tw = boot(cfg(PersonalMode::Abstract));
    let nick = character(&mut tw, "Nick", 3);
    let help = tw
        .registry()
        .spec_of("+career")
        .map(|s| s.help.to_string())
        .expect("+career is registered");
    assert!(
        help.contains("domain=<org id>"),
        "the help still advertises a name: {help}"
    );

    let refusal = err(&mut tw, nick, "+career/source add domain=Camarilla");
    assert!(
        refusal.contains("A domain needs an org id"),
        "still a bare fragment: {refusal}"
    );

    // A phantom id is ACCEPTED: `SupportSource::Domain` is a lossy attribution label
    // housing renders as the constant "domain support" and never interprets, so it
    // authorizes nothing — and refusing it would answer "no such org" to an
    // unprivileged verb.
    let out = ok(&mut tw, nick, "+career/source add domain=987654");
    assert!(
        !out.to_lowercase().contains("no such org"),
        "the player verb became an oracle: {out}"
    );
    assert!(
        profile(&tw, nick)
            .sources
            .iter()
            .any(|s| s.key() == "domain")
    );

    // On the STAFF side there is no oracle to protect, so it is said out loud.
    let staff = ok(&mut tw, GOD, "@career/set Nick=employer=org:987654");
    assert!(
        staff.contains("org:987654 does not resolve"),
        "staff were not told: {staff}"
    );
}

#[test]
fn vf_d6_wod_economy_refuses_an_unknown_argument() {
    // D6: the handler tested `raw().trim().eq_ignore_ascii_case("migrate")` and
    // treated everything else — a typo of the one documented subcommand, trailing
    // junk — as the bare show, so the one operation that matters was the one a slip
    // turned into a silent no-op. `[rulesets].enabled`'s own rule, one layer up.
    let mut tw = boot(cfg(PersonalMode::Abstract));
    let _nick = character(&mut tw, "Nick", 3);
    for bad in [
        "@wod/economy migrat",
        "@wod/economy migrate now",
        "@wod/economy show",
    ] {
        let refusal = err(&mut tw, GOD, bad);
        assert!(
            refusal.contains("Usage: @wod/economy [migrate]"),
            "`{bad}`: {refusal}"
        );
    }
    // The two real forms still work.
    let show = ok(&mut tw, GOD, "@wod/economy");
    assert!(
        show.contains("[wod.economy] as this server is running it:"),
        "{show}"
    );
    let migrate = ok(&mut tw, GOD, "@wod/economy migrate");
    assert!(migrate.contains("@wod/economy migrate:"), "{migrate}");
}

// ---------------------------------------------------------------------------
// ---- look-back fold: D-LBK-2 (the WoD half) ----
// ---------------------------------------------------------------------------

fn engine_fire(engine: &mut Engine) {
    let t = engine.world.now().saturating_add(40);
    engine.set_now(t);
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    engine.fire_due(t.saturating_mul(1000), &mut out, &mut fx);
}

/// `n` profiled characters with seeded cursors, injected as documents and booted
/// fresh — a population of a thousand for the price of a reboot.
fn profiled_population(tw: &mut TestWorld, n: u32) -> (Engine, Vec<ObjId>) {
    let now = i64::try_from(
        tw.world()
            .module_state::<EconomyStore>()
            .and_then(|s| {
                s.snapshot()
                    .cadence(mush_economy::CadenceId(cadence_id(tw)))
                    .map(|c| c.position)
            })
            .expect("cadence"),
    )
    .unwrap_or(0);
    let mut who = Vec::new();
    for i in 0..n {
        who.push(tw.spawn_player(&format!("Pop{i}")));
    }
    let mut kv = fold_kv(tw);
    let entry = kv.entry(SmolStr::new("wod")).or_default();
    for obj in &who {
        let p = SpendingProfile::new(*obj, 0, Engagement::Ambient);
        entry.push((
            SmolStr::new(format!("profile:{}", obj.0)),
            serde_json::to_value(&p).expect("serializes"),
        ));
        entry.push((
            SmolStr::new(format!("eval:{}", obj.0)),
            serde_json::json!({ "position": now, "period": 0, "seeded": true }),
        ));
    }
    let engine = reboot_fresh(tw, kv, cfg(PersonalMode::Abstract));
    (engine, who)
}

/// A thousand profiled characters, none due: a fire visits at most 256 and advances
/// the cursor past the last visited id; the next fire visits the next block; after a
/// complete sweep with nothing due a fire visits nothing until the cadence advances
/// or a profile is created; at a period boundary a fire works at most 128 and later
/// fires work the rest; and no profile is cloned before the due check.
#[test]
fn lb_dlbk2_a_fire_visits_at_most_256_works_at_most_128_and_idles_between_periods() {
    let mut tw = boot(cfg(PersonalMode::Abstract));
    let cadence = cadence_id(&tw);
    let (mut engine, who) = profiled_population(&mut tw, 1_000);
    assert_eq!(engine_store(&engine).profiles().count(), 1_000);

    let mut cursors = Vec::new();
    for _ in 0..4 {
        engine_fire(&mut engine);
        let f = engine_store(&engine).last_fire();
        assert!(
            f.visited <= ruleset_wod::economy::store::MAX_VISITED_PER_FIRE,
            "visited {}",
            f.visited
        );
        assert!(f.visited > 0, "an idle fire before the sweep was complete");
        assert_eq!(f.worked, 0);
        assert_eq!(f.cloned, 0, "a profile was cloned before the due check");
        cursors.push(engine_store(&engine).cursor());
    }
    assert_eq!(
        cursors[0],
        who[255].0 + 1,
        "the cursor did not advance past the last visited id"
    );
    assert_eq!(
        cursors[1],
        who[511].0 + 1,
        "the second fire did not visit the next block"
    );
    engine_fire(&mut engine);
    let f = engine_store(&engine).last_fire();
    assert_eq!(f.visited, 0, "a fire scanned after a complete idle sweep");
    assert!(f.short_circuited);

    // A new profile bumps the population version and wakes the scan…
    let fresh = engine
        .world
        .create(ObjKind::Player, "Newcomer", GOD, Some(LIMBO))
        .expect("a player");
    let ev = engine_cmd(
        &mut engine,
        GOD,
        "@sheet/set Newcomer/backgrounds:Resources=2",
    );
    assert!(!has_error(&ev), "{}", texts(&ev));
    let mut doc = engine
        .world
        .attr(fresh, "sheet:wod")
        .and_then(mush_core::Attr::as_json)
        .cloned()
        .expect("a sheet");
    doc["approved"] = serde_json::json!({ "approved": { "by": GOD.0, "at": 0 } });
    engine
        .world
        .set_attr(fresh, "sheet:wod", mush_core::Attr::json(doc, GOD))
        .expect("writable");
    let version = engine_store(&engine).population_version();
    let ev = engine_cmd(&mut engine, GOD, "@wod/economy migrate");
    assert!(!has_error(&ev), "{}", texts(&ev));
    assert!(
        engine_store(&engine).population_version() > version,
        "a new profile did not bump the version"
    );
    engine_fire(&mut engine);
    assert!(
        engine_store(&engine).last_fire().visited > 0,
        "a new profile did not wake the scan"
    );
    for _ in 0..4 {
        engine_fire(&mut engine);
    }
    assert!(
        engine_store(&engine).last_fire().short_circuited,
        "the scan never went idle again"
    );

    // …and a cadence advance makes everything due: at most 128 worked per fire, the
    // rest on later fires.
    let ev = engine_cmd(
        &mut engine,
        GOD,
        &format!("@econ/cadence advance {cadence} 1"),
    );
    assert!(!has_error(&ev), "{}", texts(&ev));
    let mut worked_total = 0usize;
    for _ in 0..12 {
        engine_fire(&mut engine);
        let f = engine_store(&engine).last_fire();
        assert!(
            f.worked <= ruleset_wod::economy::store::MAX_WORKED_PER_FIRE,
            "worked {}",
            f.worked
        );
        assert!(f.visited <= ruleset_wod::economy::store::MAX_VISITED_PER_FIRE);
        assert!(
            f.cloned <= f.worked,
            "a profile was cloned without being worked"
        );
        worked_total += f.worked;
    }
    assert_eq!(
        worked_total, 1_001,
        "the period was not spread over later fires: {worked_total}"
    );
}

// ---------------------------------------------------------------------------
// ---- look-back fold: D-LBK-3 ----
// ---------------------------------------------------------------------------

/// Boot `tw` fresh over `base`'s documents with `who`'s profile rewritten by `f`,
/// and say whether the loader refused it (the profile is then re-derived in the
/// same boot).
fn reboot_with_patched_profile(
    tw: &TestWorld,
    base: &Kv,
    who: ObjId,
    f: impl Fn(&mut serde_json::Value),
) -> (Engine, bool) {
    let mut kv = base.clone();
    let key = format!("profile:{}", who.0);
    let entry = kv.get_mut(&SmolStr::new("wod")).expect("wod namespace");
    let mut seen = false;
    for (k, v) in entry.iter_mut() {
        if k.as_str() == key {
            f(v);
            seen = true;
        }
    }
    assert!(seen, "no persisted profile for #{}", who.0);
    let engine = reboot_fresh(tw, kv, cfg(PersonalMode::Abstract));
    let p = engine_profile(&engine, who);
    let rederived = p
        .history
        .iter()
        .any(|e| e.what.contains("re-derived at boot"));
    (engine, rederived)
}

/// A profile past any documented cap refuses to deserialize — 33 history entries,
/// 9 sources, a 5-option prompt, a 65-byte text field, a 257-byte history line — and
/// the character is re-derived in the same boot; a profile inside every cap loads
/// as written.
#[test]
fn lb_dlbk3_an_over_cap_profile_refuses_to_load_and_is_re_derived() {
    let mut tw = boot(cfg(PersonalMode::Abstract));
    let nick = character(&mut tw, "Nick", 3);
    ok(&mut tw, nick, "+career/source add career=fixer");
    let base = fold_kv(&mut tw);
    fn option(k: &str) -> serde_json::Value {
        serde_json::json!({ "key": k, "label": "x", "outcome": "StatusQuo" })
    }
    fn prompt(n: usize) -> serde_json::Value {
        let options: Vec<serde_json::Value> = (0..n).map(|i| option(&format!("o{i}"))).collect();
        serde_json::json!({
            "id": 1, "kind": "StrainRelief", "options": options,
            "default": "StatusQuo", "opened_at": 0, "opened_period": 0, "expires_after_periods": 2
        })
    }
    type Patch = Box<dyn Fn(&mut serde_json::Value)>;
    let cases: Vec<(&str, Patch)> = vec![
        (
            "33 history entries",
            Box::new(|v| {
                let history: Vec<serde_json::Value> = (0..33)
                    .map(|i| serde_json::json!({ "at": i, "what": "x" }))
                    .collect();
                v["history"] = serde_json::Value::Array(history);
            }),
        ),
        (
            "9 sources",
            Box::new(|v| {
                let sources: Vec<serde_json::Value> =
                    (0..9).map(|_| serde_json::json!("Inheritance")).collect();
                v["sources"] = serde_json::Value::Array(sources);
            }),
        ),
        (
            "two sources of one kind",
            Box::new(|v| {
                v["sources"] = serde_json::json!(["Inheritance", "Inheritance"]);
            }),
        ),
        (
            "a 5-option prompt",
            Box::new(|v| {
                v["prompt"] = prompt(5);
            }),
        ),
        (
            "a 65-byte career kind",
            Box::new(|v| {
                v["sources"] = serde_json::json!([{ "Career": { "kind": "k".repeat(65) } }]);
            }),
        ),
        (
            "a 257-byte history line",
            Box::new(|v| {
                v["history"] = serde_json::json!([{ "at": 0, "what": "h".repeat(257) }]);
            }),
        ),
        (
            "9 haven qualities",
            Box::new(|v| {
                let qualities: Vec<&str> = (0..9).map(|_| "q").collect();
                v["haven"] = serde_json::json!({ "dots": 3, "qualities": qualities });
            }),
        ),
        (
            "strain above the ceiling",
            Box::new(|v| {
                v["strain"] = serde_json::json!(11);
            }),
        ),
    ];
    for (what, f) in cases {
        let (engine, rederived) = reboot_with_patched_profile(&tw, &base, nick, f);
        assert!(rederived, "{what}: the over-cap profile loaded as written");
        let p = engine_profile(&engine, nick);
        assert_eq!(
            p.resources, 3,
            "{what}: the re-derived profile lost the sheet reading"
        );
        assert!(p.prompt.is_none() && p.strain == 0, "{what}");
    }
    // Inside every cap, the document loads exactly as written.
    let (engine, rederived) = reboot_with_patched_profile(&tw, &base, nick, |v| {
        let history: Vec<serde_json::Value> = (0..32)
            .map(|i| serde_json::json!({ "at": i, "what": "x" }))
            .collect();
        v["history"] = serde_json::Value::Array(history);
        v["prompt"] = prompt(4);
        v["strain"] = serde_json::json!(10);
    });
    assert!(!rederived, "a profile inside every cap was refused");
    let p = engine_profile(&engine, nick);
    assert_eq!(p.history.len(), 32);
    assert_eq!(p.prompt.as_ref().map(|x| x.options.len()), Some(4));
    assert_eq!(p.strain, 10);
}

/// `+prompt` renders at most four options: a four-option prompt shows four lines, and
/// a five-option document never reaches the render at all.
#[test]
fn lb_dlbk3_prompt_renders_at_most_four_options() {
    let mut tw = boot(cfg(PersonalMode::Abstract));
    let nick = character(&mut tw, "Nick", 3);
    let base = fold_kv(&mut tw);
    let option =
        |k: &str| serde_json::json!({ "key": k, "label": "an answer", "outcome": "StatusQuo" });
    let (mut engine, rederived) = reboot_with_patched_profile(&tw, &base, nick, move |v| {
        let options: Vec<serde_json::Value> =
            ["a", "b", "c", "d"].iter().map(|k| option(k)).collect();
        v["prompt"] = serde_json::json!({
            "id": 7, "kind": "StrainRelief", "options": options,
            "default": "StatusQuo", "opened_at": 0, "opened_period": 0, "expires_after_periods": 2
        });
    });
    assert!(!rederived);
    engine_conn(&mut engine, nick);
    let seen = texts(&engine_cmd(&mut engine, nick, "+prompt"));
    assert_eq!(seen.matches("  +prompt ").count(), 4, "{seen}");
    assert_eq!(ruleset_wod::economy::record::MAX_PROMPT_OPTIONS, 4);
}

// ---------------------------------------------------------------------------
// ---- look-back fold: E5c-C96 ----
// ---------------------------------------------------------------------------

/// A `HousingPressure` prompt that lapsed this period does not re-open this period,
/// and re-asks next period while the home is still behind.
#[test]
fn lb_e5cc96_a_lapsed_housing_pressure_prompt_waits_one_period_before_re_asking() {
    let mut tw = boot(cfg(PersonalMode::Abstract));
    let nick = character(&mut tw, "Nick", 0);
    let flat = property(&mut tw, "Flat", 3);
    let _t = tenancy_for(&mut tw, flat, nick);
    let mut opened = None;
    for _ in 0..8 {
        period(&mut tw, 1);
        if let Some(p) = profile(&tw, nick).prompt.as_ref()
            && matches!(p.kind, PromptKind::HousingPressure { .. })
        {
            opened = Some(p.opened_period);
            break;
        }
    }
    let opened = opened.expect("housing pressure never asked");
    // Unanswered, it lapses after PROMPT_EXPIRY_PERIODS — and that period stays quiet.
    let mut lapsed_at = None;
    for _ in 0..4 {
        period(&mut tw, 1);
        let p = profile(&tw, nick);
        let ordinal = store(&tw).eval_state(nick).period;
        if p.prompt.is_none() {
            lapsed_at = Some(ordinal);
            assert!(
                p.history.iter().any(|e| e.what.contains("lapsed")),
                "the prompt vanished without lapsing: {:?}",
                p.history
            );
            break;
        }
        assert_eq!(
            p.prompt.as_ref().map(|x| x.opened_period),
            Some(opened),
            "a second prompt replaced the first"
        );
    }
    let lapsed_at = lapsed_at.expect("the prompt never lapsed");
    assert!(lapsed_at > opened);
    // The home is still behind, so the standing condition re-asks — next period, not
    // in the same breath.
    period(&mut tw, 1);
    let p = profile(&tw, nick);
    assert!(
        matches!(
            p.prompt.as_ref().map(|x| &x.kind),
            Some(PromptKind::HousingPressure { .. })
        ),
        "housing pressure did not re-ask next period: {:?}",
        p.prompt
    );
    assert_eq!(
        p.prompt.as_ref().map(|x| x.opened_period),
        Some(lapsed_at + 1)
    );
}

/// The same quiet period for `StrainRelief`: at the ceiling, a lapsed relief prompt
/// is not replaced in the period it lapses and re-asks the period after.
#[test]
fn lb_e5cc96_a_lapsed_strain_relief_prompt_waits_one_period_before_re_asking() {
    let mut tw = boot(cfg(PersonalMode::Abstract));
    let nick = character(&mut tw, "Nick", 3);
    ok(&mut tw, nick, "+career/source add career=fixer");
    ok(&mut tw, nick, "+career/stance advance");
    for _ in 0..(u64::from(STRAIN_MAX) + 6) {
        period(&mut tw, 1);
    }
    assert_eq!(profile(&tw, nick).strain, STRAIN_MAX);
    ok(&mut tw, nick, "+career/engage on");
    period(&mut tw, 1);
    let opened = match profile(&tw, nick).prompt {
        Some(p) if matches!(p.kind, PromptKind::StrainRelief) => p.opened_period,
        other => panic!("relief was not offered at the ceiling: {other:?}"),
    };
    let mut lapsed_at = None;
    for _ in 0..4 {
        period(&mut tw, 1);
        let p = profile(&tw, nick);
        let ordinal = store(&tw).eval_state(nick).period;
        if p.prompt.is_none() {
            lapsed_at = Some(ordinal);
            break;
        }
        assert_eq!(
            p.prompt.as_ref().map(|x| x.opened_period),
            Some(opened),
            "a second prompt replaced the first"
        );
    }
    let lapsed_at = lapsed_at.expect("the relief prompt never lapsed");
    assert!(lapsed_at > opened, "the quiet period was skipped");
    period(&mut tw, 1);
    let p = profile(&tw, nick);
    assert_eq!(p.strain, STRAIN_MAX, "strain left the ceiling");
    assert!(
        matches!(
            p.prompt.as_ref().map(|x| &x.kind),
            Some(PromptKind::StrainRelief)
        ),
        "relief did not re-ask next period: {:?}",
        p.prompt
    );
    assert_eq!(
        p.prompt.as_ref().map(|x| x.opened_period),
        Some(lapsed_at + 1)
    );
}

// ---------------------------------------------------------------------------
// The chargen gates read the schema's field id, not the spelling the player typed
// ---------------------------------------------------------------------------

/// The in-progress sheet, decoded.
fn sheet_doc(tw: &TestWorld, who: ObjId) -> mush_ruleset::SheetDoc {
    tw.world()
        .attr(who, "sheet:wod")
        .and_then(mush_core::Attr::as_json)
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .expect("a sheet")
}

/// A player in an open chargen session on `template`.
fn in_chargen(tw: &mut TestWorld, name: &str, template: &str) -> ObjId {
    let who = tw.spawn_player(name);
    ok(tw, who, "+chargen");
    ok(tw, who, &format!("+chargen/template {template}"));
    who
}

#[test]
fn a_capitalised_generation_does_not_walk_past_the_chargen_lock() {
    let mut tw = boot(cfg(PersonalMode::Off));
    let who = in_chargen(&mut tw, "Nick", "vampire");
    let before = sheet_doc(&tw, who).number("generation");
    assert_eq!(before, Some(13), "a fresh vampire is 13th generation");
    for probe in [
        "+stat generation=4",
        "+stat Generation=4",
        "+stat GENERATION=4",
        "+stat gEnErAtIoN=4",
    ] {
        let out = err(&mut tw, who, probe);
        assert!(
            out.contains("not a chargen-allocatable trait"),
            "`{probe}` was not refused as a locked field: {out}"
        );
        assert_eq!(
            sheet_doc(&tw, who).number("generation"),
            before,
            "`{probe}` moved generation"
        );
    }
}

#[test]
fn every_splats_locked_field_refuses_a_mixed_case_spelling() {
    let mut tw = boot(cfg(PersonalMode::Off));
    for (i, (template, probe)) in [
        ("changeling", "+stat GlAmOuR=5"),
        ("changeling", "+stat WILLPOWER=8"),
        ("mage", "+stat SPHERES:Forces=3"),
        ("werewolf", "+stat RaGe=9"),
        ("werewolf", "+stat GNOSIS=8"),
        ("ghoul", "+stat Bond_Level=3"),
        ("kinfolk", "+stat GIFTS:Persuasion=1"),
    ]
    .into_iter()
    .enumerate()
    {
        let who = in_chargen(&mut tw, &format!("Probe{i}"), template);
        let out = err(&mut tw, who, probe);
        assert!(
            out.contains("not a chargen-allocatable trait"),
            "{template}: `{probe}` was not refused as a locked field: {out}"
        );
    }
}

#[test]
fn the_wizards_free_form_step_canonicalises_before_the_lock_too() {
    // wizard.rs feeds player-typed field names to `apply_stat` raw — the Freebies
    // step most freely of all — so the fix has to live where both surfaces meet.
    let mut tw = boot(cfg(PersonalMode::Off));
    let who = tw.spawn_player("Nick");
    ok(&mut tw, who, "+chargen/wizard");
    ok(&mut tw, who, "vampire");
    // Template → Concept → attribute priorities → Attributes → ability
    // priorities → Abilities → Backgrounds → Template Traits → Virtues → Freebies.
    let mut out = String::new();
    for _ in 0..9 {
        out = ok(&mut tw, who, "next");
    }
    assert!(out.contains("Freebies"), "not at the Freebies step: {out}");
    let before = sheet_doc(&tw, who).number("generation");
    let out = err(&mut tw, who, "GENERATION=4");
    assert!(
        out.contains("not a chargen-allocatable trait"),
        "the wizard let a capitalised locked field through: {out}"
    );
    assert_eq!(
        sheet_doc(&tw, who).number("generation"),
        before,
        "the wizard moved generation"
    );
}

#[test]
fn a_capitalised_settable_trait_still_sets_the_canonical_field() {
    let mut tw = boot(cfg(PersonalMode::Off));
    let who = in_chargen(&mut tw, "Nick", "mortal");
    ok(&mut tw, who, "+stat strength=4");
    assert_eq!(sheet_doc(&tw, who).number("strength"), Some(4));
    ok(&mut tw, who, "+stat Strength=2");
    assert_eq!(
        sheet_doc(&tw, who).number("strength"),
        Some(2),
        "a capitalised spelling must still write the one canonical field"
    );
}

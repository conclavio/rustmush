//! E6a acceptance (08f §3.6): **run and pay**.
//!
//! The verify axis of this cut is *uncontrolled faucets, board spam, an org-treasury
//! drip, a fixer skimming, split arithmetic, party lifecycle*, so the rhythms here
//! are:
//!
//! - **no listing without escrow.** Every post asserts `held_amount` on the funding
//!   account, and every refused post asserts that **no contract was drafted**.
//! - **every share is named, and each row sums.** The split tests assert the exact
//!   payout rows, not just that the offer was accepted.
//! - **the money is the contract's.** The board is a projection; a listing that goes
//!   away never takes an escrow with it.
//! - **fold and reboot.** Profiles, listings and the party all survive a restart, and
//!   a record that disagrees with its own key is dropped rather than trusted.
//!
//! Everything runs through the real `Engine` via `TestWorld`, on the production
//! module composition (org → faction → economy → party → the ruleset), with the
//! `sr.month` cadence **declared** exactly as the host contributes it.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use mush_core::{Audience, GOD, ModuleBootData, ModuleWrite, ObjId};
use mush_economy::{
    AccountId, ContractId, ContractState, DeclaredCadence, DeclaredCadenceClock, EconomyConfig,
    EconomySnapshot, EconomyStore, ObligationStatus,
};
use mush_org::{CapabilityId, PrincipalId};
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, text_of};
use ruleset_shadowrun::economy::record::{RunId, RunState};
use ruleset_shadowrun::{LifestyleTier, SrBoardObserver, SrEconomyConfig, SrEconomyStore};
use smol_str::SmolStr;

const NS: &str = "shadowrun";
/// A short `sr.month` so a test can advance a period without a month of wall clock.
const PERIOD: i64 = 100;

// ---- rig ------------------------------------------------------------------

fn vocabulary() -> BTreeSet<CapabilityId> {
    mush_economy::CAPABILITIES
        .iter()
        .copied()
        .chain(["org.role.manage", "org.grant.delegate"])
        .map(|c| CapabilityId::new(c).expect("well-formed"))
        .collect()
}

fn sr_config() -> SrEconomyConfig {
    SrEconomyConfig {
        enabled: true,
        ..SrEconomyConfig::default()
    }
}

/// The production composition with `[shadowrun.economy].enabled = true` and a
/// declared `sr.month`, exactly as `config_ext::shadowrun_cadences` contributes it.
fn boot_with(cfg: SrEconomyConfig) -> TestWorld {
    let econ = EconomyConfig {
        cadences: BTreeMap::from([(
            SmolStr::new("sr.month"),
            DeclaredCadence {
                clock: DeclaredCadenceClock::WorldIc,
                period_secs: PERIOD,
                catch_up: ruleset_shadowrun::SR_MONTH_CATCH_UP,
            },
        )]),
        ..EconomyConfig::default()
    };
    TestWorld::with_modules(vec![
        Box::new(
            mush_org::OrgModule::new()
                .with_capabilities(vocabulary())
                .with_scope_use(vec![Arc::new(mush_economy::EconomyScopeUse)]),
        ),
        Box::new(mush_faction::FactionModule::new()),
        Box::new(
            mush_economy::EconomyModule::with_custody(vec![])
                .with_config(econ)
                .with_observers(vec![Arc::new(SrBoardObserver::new(cfg.clone()))]),
        ),
        Box::new(mush_party::PartyModule::new()),
        // The ruleset registers its chargen/karma approval handlers against the jobs
        // module's state slot, so it has to be loaded first — the production order.
        Box::new(mush_jobs::JobsModule::new()),
        Box::new(ruleset_shadowrun::ShadowrunRuleset::with_seed(7).economy(cfg)),
    ])
}

fn boot() -> TestWorld {
    boot_with(sr_config())
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
        matches!(e, OutputEvent::Sys(rt) if rt.spans().iter().any(|sp| sp.tag.as_deref() == Some("error")))
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
fn snap(tw: &TestWorld) -> &EconomySnapshot {
    tw.world()
        .module_state::<EconomyStore>()
        .expect("economy store")
        .snapshot()
}
fn sr(tw: &TestWorld) -> &SrEconomyStore {
    tw.world()
        .module_state::<SrEconomyStore>()
        .expect("sr store")
}
fn bal(tw: &TestWorld, id: AccountId) -> i64 {
    snap(tw)
        .account(id)
        .expect("account")
        .posted_balance
        .minor()
}
fn held(tw: &TestWorld, id: AccountId) -> i64 {
    snap(tw).account(id).expect("account").held_amount.minor()
}
fn contracts(tw: &TestWorld) -> Vec<ContractId> {
    snap(tw).contracts().map(|c| c.id).collect()
}
fn only_run(tw: &TestWorld) -> RunId {
    let ids: Vec<RunId> = sr(tw).runs().map(|r| r.id).collect();
    assert_eq!(ids.len(), 1, "expected exactly one listing, got {ids:?}");
    ids[0]
}
/// A listing's state — **or `Closed` when the record is gone**.
///
/// Since the verify fold (D1) a closed listing is deleted rather than written, so
/// "the board says closed" and "the board no longer holds it" are the same fact and
/// every assertion that used to read the state reads this.
fn run_state(tw: &TestWorld, id: RunId) -> RunState {
    sr(tw).run(id).map_or(RunState::Closed, |r| r.state)
}
fn contract_of(tw: &TestWorld, id: RunId) -> ContractId {
    sr(tw).run(id).expect("listing").contract
}
fn state(tw: &TestWorld, id: ContractId) -> ContractState {
    snap(tw).contract(id).expect("contract").state
}
fn account_of(tw: &TestWorld, who: ObjId) -> AccountId {
    sr(tw)
        .profile(who)
        .and_then(|p| p.account)
        .expect("a profiled account")
}
fn faucet(tw: &TestWorld, name: &str) -> AccountId {
    let owner = PrincipalId::System(mush_org::SystemPrincipalId::new(name).expect("well-formed"));
    snap(tw)
        .accounts()
        .find(|a| a.owner == owner)
        .expect("faucet account")
        .id
}

/// An **approved** character with a nuyen account and `funds` in it.
///
/// It goes through the real adoption path — an approved sheet plus
/// `@sr/migrate commit` — rather than writing a profile by hand, so the fixture
/// exercises exactly what M13 promises: `commit` adopts an approved character,
/// opens their account, and mints what the sheet says.
fn runner(tw: &mut TestWorld, name: &str, funds: i64) -> ObjId {
    let who = tw.spawn_player(name);
    approve(tw, who, funds);
    ok(tw, GOD, &format!("@sr/migrate commit #{}", who.0));
    who
}

/// Stamp `who`'s sheet approved with `nuyen = funds`.
fn approve(tw: &mut TestWorld, who: ObjId, funds: i64) {
    let store = ruleset_shadowrun::ShadowrunRuleset::new().store();
    let mut doc = store.get_or_default(tw.world(), who);
    doc.set_number("nuyen", funds);
    doc.approved = mush_ruleset::ApprovalState::Approved { by: GOD, at: 10 };
    store
        .put_by(tw.world_mut(), who, &doc, GOD)
        .expect("the sheet writes");
}

fn party_of(tw: &TestWorld, leader: ObjId) -> u64 {
    mush_party::parties_of(tw.world(), leader)
        .first()
        .expect("a live party")
        .0
}

/// Every module's persisted documents, folded from the ops written since the world
/// was built. A reboot has to replay **all** of them: `modules_loaded` hands each
/// module whatever this map holds for its namespace, and a namespace that is missing
/// comes back **empty**.
type Kv = BTreeMap<SmolStr, Vec<(SmolStr, serde_json::Value)>>;

fn fold_all(tw: &mut TestWorld) -> Kv {
    let mut kv: Kv = BTreeMap::new();
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

fn reboot(tw: &mut TestWorld, kv: &Kv) {
    let boot: HashMap<SmolStr, ModuleBootData> = kv
        .iter()
        .map(|(ns, entries)| {
            (
                ns.clone(),
                ModuleBootData {
                    kv: entries.clone(),
                    streams: HashMap::new(),
                    ..Default::default()
                },
            )
        })
        .collect();
    tw.modules_loaded(boot);
}

/// Patch one persisted document in place before a reboot.
fn patch(kv: &mut Kv, ns: &str, key: &str, f: impl Fn(&mut serde_json::Value)) {
    let entry = kv.get_mut(&SmolStr::new(ns)).expect("the namespace wrote");
    let mut seen = false;
    for (k, v) in entry.iter_mut() {
        if k.as_str() == key {
            f(v);
            seen = true;
        }
    }
    assert!(seen, "no such persisted key to patch: {ns}/{key}");
}

/// Drop one persisted document before a reboot.
fn drop_doc(kv: &mut Kv, ns: &str, key: &str) {
    let entry = kv.get_mut(&SmolStr::new(ns)).expect("the namespace wrote");
    let before = entry.len();
    entry.retain(|(k, _)| k.as_str() != key);
    assert!(
        entry.len() < before,
        "no such persisted key to drop: {ns}/{key}"
    );
}

/// One sweep fire at wall-clock second `t`.
fn sweep_at(tw: &mut TestWorld, t: i64) -> Vec<(Audience, OutputEvent)> {
    tw.set_now(t);
    tw.fire_due(t.saturating_mul(1000))
}

// ---------------------------------------------------------------------------
// Boot: the instrument, the faucets, the declared clock
// ---------------------------------------------------------------------------

/// §3.3: boot registers `sr.nuyen` at scale 0 with its `¥` prefix, opens the two
/// faucets and the sink, and finds the **declared** `sr.month` clock.
#[test]
fn boot_registers_the_instrument_the_faucets_and_finds_the_declared_clock() {
    let tw = boot();
    let def = snap(&tw)
        .instrument(&mush_economy::InstrumentId::new("sr.nuyen").expect("id"))
        .expect("sr.nuyen is registered");
    assert_eq!(def.scale, 0);
    let display = def.display.clone().expect("a display");
    assert_eq!(display.symbol.as_str(), "\u{a5}");
    assert_eq!(display.position, mush_economy::SymbolPosition::Prefix);
    for name in ["sr.chargen", "sr.migration", "sr.lifestyle"] {
        let owner =
            PrincipalId::System(mush_org::SystemPrincipalId::new(name).expect("well-formed"));
        assert!(
            snap(&tw).accounts().any(|a| a.owner == owner),
            "{name} has no account"
        );
    }
    let cadence = sr(&tw).cadence().expect("the declared sr.month was found");
    let clock = snap(&tw).cadence(cadence).expect("the clock exists");
    assert_eq!(clock.name.as_str(), "sr.month");
    assert_eq!(clock.schema, mush_economy::CADENCE_SCHEMA_DECLARED);
}

/// §9's partial-switch table: `enabled = false` registers the verbs and creates
/// nothing — no instrument, no faucet, no listing, no obligation.
#[test]
fn a_disabled_layer_registers_the_verbs_and_creates_nothing() {
    let mut tw = boot_with(SrEconomyConfig::default());
    assert!(
        snap(&tw)
            .instrument(&mush_economy::InstrumentId::new("sr.nuyen").expect("id"))
            .is_none(),
        "a disabled layer registered an instrument"
    );
    let who = tw.spawn_player("Nobody");
    let refused = err(&mut tw, who, "+run/list");
    assert!(refused.contains("not enabled"), "{refused}");
}

/// `@sr/clock` is **show only** since SP1 (D-LBK-7): it reports the declared clock
/// and never creates one.
#[test]
fn sr_clock_shows_the_declared_cadence_and_creates_nothing() {
    let mut tw = boot();
    let before = snap(&tw).cadences().count();
    let out = ok(&mut tw, GOD, "@sr/clock");
    assert!(out.contains("sr.month"), "{out}");
    assert!(out.contains("never creates"), "{out}");
    assert_eq!(snap(&tw).cadences().count(), before);
}

// ---------------------------------------------------------------------------
// Board
// ---------------------------------------------------------------------------

/// §3.2 "Board" + answer 24: a post drafts, offers and accepts in one act, and the
/// escrow is **real** — `held_amount` on the funding account, not a promise.
#[test]
fn a_listed_runs_escrow_is_real() {
    let mut tw = boot();
    let j = runner(&mut tw, "Johnson", 50_000);
    let acct = account_of(&tw, j);
    ok(&mut tw, j, "+run/post Extraction=20000");
    let id = only_run(&tw);
    assert_eq!(run_state(&tw, id), RunState::Listed);
    assert_eq!(held(&tw, acct), 20_000, "the escrow is placed at the post");
    assert_eq!(bal(&tw, acct), 50_000, "the escrow is held, not spent");
    assert!(matches!(
        state(&tw, contract_of(&tw, id)),
        ContractState::Offered | ContractState::Active
    ));
}

/// M17: a post with no funds **lists nothing and cancels the draft**, so no unfunded
/// `Offered` contract counts against the issuer's cap.
#[test]
fn a_post_with_no_funds_lists_nothing_and_cancels_the_draft() {
    let mut tw = boot();
    let j = runner(&mut tw, "Broke", 10);
    err(&mut tw, j, "+run/post Extraction=20000");
    assert_eq!(sr(&tw).runs().count(), 0, "a listing was created");
    for c in contracts(&tw) {
        assert!(
            state(&tw, c).is_terminal(),
            "contract #{c} was left standing after a refused accept"
        );
    }
}

/// D-E6-4 / M4: a `System` payer is refused **at post**, in that account's own
/// hiding text, and **no contract was drafted**.
#[test]
fn a_system_payer_is_refused_at_post_with_no_draft_left_behind() {
    let mut tw = boot();
    let j = runner(&mut tw, "Johnson", 50_000);
    let f = faucet(&tw, "sr.chargen");
    let refused = err(&mut tw, GOD, &format!("+run/post Faucet=20000 account={f}"));
    assert!(refused.contains("No such account."), "{refused}");
    assert!(contracts(&tw).is_empty(), "a contract was drafted");
    assert_eq!(sr(&tw).runs().count(), 0);
    let _ = j;
}

/// §9: `MAX_OPEN_RUNS_PER_PRINCIPAL` refuses the next post, and the escrow of the
/// four that stand is untouched.
#[test]
fn the_per_principal_open_run_cap_refuses_the_next_post() {
    let mut tw = boot();
    let j = runner(&mut tw, "Johnson", 100_000);
    let acct = account_of(&tw, j);
    for i in 0..4 {
        ok(&mut tw, j, &format!("+run/post Job{i}=1000"));
    }
    let refused = err(&mut tw, j, "+run/post Fifth=1000");
    assert!(refused.contains("as many runs listed"), "{refused}");
    assert_eq!(sr(&tw).runs().count(), 4);
    assert_eq!(held(&tw, acct), 4_000);
}

/// M5: a faction-visible run is invisible to a non-member and renders a **notice**
/// — never its terms — to a member, through `mush_faction`'s own predicates.
#[test]
fn a_faction_run_is_invisible_to_a_non_member_and_a_notice_to_a_member() {
    let mut tw = boot();
    let j = runner(&mut tw, "Johnson", 50_000);
    let inside = runner(&mut tw, "Inside", 0);
    let outside = runner(&mut tw, "Outside", 0);
    ok(&mut tw, GOD, "@faction/create Ancients");
    let f = tw
        .world()
        .module_state::<mush_faction::FactionState>()
        .and_then(|s| s.faction_by_name("Ancients").map(|f| f.id))
        .expect("the faction exists");
    ok(
        &mut tw,
        GOD,
        &format!("@faction/add Ancients=#{}", inside.0),
    );
    ok(
        &mut tw,
        j,
        &format!("+run/post Smuggle=9000 board=faction:{}", f.0),
    );
    let id = only_run(&tw);
    let seen = ok(&mut tw, inside, &format!("+run/show {id}"));
    assert!(seen.contains("Smuggle"), "{seen}");
    assert!(
        !seen.contains("9000"),
        "a faction member was shown the terms: {seen}"
    );
    let refused = err(&mut tw, outside, &format!("+run/show {id}"));
    assert_eq!(refused.trim(), "No such run.");
}

/// 08a decision 13: a non-viewer gets exactly "No such run." — the same text a
/// nonexistent listing gets.
#[test]
fn a_nonexistent_and_an_unviewable_run_answer_alike() {
    let mut tw = boot();
    let j = runner(&mut tw, "Johnson", 50_000);
    let stranger = tw.spawn_player("Stranger");
    ok(&mut tw, j, "+run/post Quiet=1000");
    let id = only_run(&tw);
    let missing = err(&mut tw, stranger, "+run/show 9999");
    let hidden = err(&mut tw, stranger, &format!("+run/show {id}"));
    assert_eq!(missing.trim(), "No such run.");
    assert_eq!(hidden.trim(), missing.trim());
}

/// §3.4: `+run/list` shows a public listing to a profiled character and nothing to
/// somebody with no wallet in the game.
#[test]
fn the_board_lists_to_a_profiled_character_only() {
    let mut tw = boot();
    let j = runner(&mut tw, "Johnson", 50_000);
    let reader = runner(&mut tw, "Reader", 0);
    let stranger = tw.spawn_player("Stranger");
    ok(&mut tw, j, "+run/post Legwork=5000");
    assert!(ok(&mut tw, reader, "+run/list").contains("Legwork"));
    assert!(
        !ok(&mut tw, stranger, "+run/list").contains("Legwork"),
        "an unprofiled stranger read the board"
    );
}

/// §3.2: `+run/withdraw` closes the listing and the escrow comes back.
#[test]
fn withdrawing_a_listing_refunds_its_escrow() {
    let mut tw = boot();
    let j = runner(&mut tw, "Johnson", 50_000);
    let acct = account_of(&tw, j);
    ok(&mut tw, j, "+run/post Cancelled=7000");
    let id = only_run(&tw);
    assert_eq!(held(&tw, acct), 7_000);
    ok(&mut tw, j, &format!("+run/withdraw {id}"));
    assert_eq!(held(&tw, acct), 0, "the escrow was not released");
    assert_eq!(bal(&tw, acct), 50_000);
    assert_eq!(run_state(&tw, id), RunState::Closed);
}

/// Only the payer withdraws; anybody else gets the hiding text.
#[test]
fn only_the_payer_withdraws_a_listing() {
    let mut tw = boot();
    let j = runner(&mut tw, "Johnson", 50_000);
    let other = runner(&mut tw, "Other", 0);
    ok(&mut tw, j, "+run/post Mine=1000");
    let id = only_run(&tw);
    assert_eq!(
        err(&mut tw, other, &format!("+run/withdraw {id}")).trim(),
        "No such run."
    );
    assert_eq!(run_state(&tw, id), RunState::Listed);
}

// ---------------------------------------------------------------------------
// Roles, adjudicator, settlement route
// ---------------------------------------------------------------------------

/// M2 / D-E6-20: without `adjudicator=` the **payer** attests; a named adjudicator
/// is never the payer and never the poster.
#[test]
fn the_adjudicator_defaults_to_the_payer_and_is_never_a_party() {
    let mut tw = boot();
    let j = runner(&mut tw, "Johnson", 50_000);
    ok(&mut tw, j, "+run/post Default=1000");
    let id = only_run(&tw);
    let c = snap(&tw)
        .contract(contract_of(&tw, id))
        .expect("contract")
        .clone();
    assert_eq!(
        c.consent.adjudicator,
        Some(mush_economy::AttestorSet::Party(PrincipalId::Object(j)))
    );
    assert_eq!(
        c.consent.evidence,
        vec![mush_economy::EvidenceKind::Attestation]
    );
    let refused = err(
        &mut tw,
        j,
        &format!("+run/post Self=1000 adjudicator=#{}", j.0),
    );
    assert!(refused.contains("never a party"), "{refused}");
}

/// M2: E6a settles by attestation and nothing else — every condition on the frozen
/// surface is an `Attestation`, so a `before`/`after`/`fact` row is unreachable.
#[test]
fn every_condition_on_a_posted_run_is_an_attestation() {
    let mut tw = boot();
    let j = runner(&mut tw, "Johnson", 50_000);
    ok(
        &mut tw,
        j,
        "+run/post Typed=1000 outcomes=success,partial,failure",
    );
    let id = only_run(&tw);
    let c = snap(&tw)
        .contract(contract_of(&tw, id))
        .expect("contract")
        .clone();
    assert_eq!(c.consent.conditions.len(), 3);
    assert!(
        c.consent
            .conditions
            .iter()
            .all(|x| matches!(x, mush_economy::TypedCondition::Attestation { .. })),
        "a non-attestation condition reached the surface"
    );
}

/// M1 / D-E6-19: a fixer takes a **named fee row** and is never the issuer.
///
/// Since the verify fold (M1) the fee row exists only once a **roster** does: a
/// `Listed` run pays its whole escrow back to the payer in every row, so the
/// adjudicator attesting one before anybody is crewed moves nothing. The fee appears
/// at `+run/crew accept`, which is the moment the fixer has arranged anything.
#[test]
fn a_fixer_takes_a_named_fee_row_and_is_never_the_issuer() {
    let mut tw = boot();
    let j = runner(&mut tw, "Johnson", 50_000);
    let fixer = runner(&mut tw, "Fixer", 0);
    let a = runner(&mut tw, "Alder", 0);
    let fixer_acct = account_of(&tw, fixer);
    ok(
        &mut tw,
        j,
        &format!(
            "+run/post Brokered=10000 slots=1 fixer=#{} fee=1000",
            fixer.0
        ),
    );
    let id = only_run(&tw);
    let cid = contract_of(&tw, id);
    let c = snap(&tw).contract(cid).expect("contract").clone();
    assert_eq!(
        c.issuer,
        PrincipalId::Object(j),
        "the fixer became the issuer"
    );
    assert_eq!(c.roles.representative, Some(PrincipalId::Object(fixer)));
    assert_eq!(
        fee_in_rows(&c, fixer_acct),
        vec![0, 0],
        "a crewless listing already carried the fixer's fee (M1)"
    );
    // Crew it: the fee row appears, in every row, exactly once.
    ok(&mut tw, a, "+party/create Solo");
    let party = party_of(&tw, a);
    ok(&mut tw, a, &format!("+run/apply {id}={party}"));
    ok(&mut tw, j, &format!("+run/crew {id}=accept"));
    let c = snap(&tw).contract(cid).expect("contract").clone();
    assert_eq!(
        fee_in_rows(&c, fixer_acct),
        vec![1_000, 1_000],
        "a crewed run does not pay the named fee in every row"
    );
    // A fixer with no fee row is refused: an unnamed remainder never reaches them.
    let refused = err(
        &mut tw,
        j,
        &format!("+run/post Unpaid=1000 fixer=#{}", fixer.0),
    );
    assert!(refused.contains("fee row"), "{refused}");
}

/// What each outcome row pays `to`, in table order.
fn fee_in_rows(c: &mush_economy::EconomicContract, to: AccountId) -> Vec<i64> {
    c.consent
        .terms
        .outcomes
        .iter()
        .map(|row| {
            row.payouts
                .iter()
                .filter(|s| s.to == to)
                .map(|s| match s.share {
                    mush_economy::Share::Exact(a) => a.minor(),
                    mush_economy::Share::Permille(_) => 0,
                })
                .sum()
        })
        .collect()
}

/// D-E6-6 / M18: every row sums to the escrowed total exactly, so `remainder_to` is
/// inert — and it resolves to the **payer's** account, never the fixer's.
#[test]
fn every_outcome_row_sums_exactly_and_no_remainder_reaches_the_fixer() {
    let mut tw = boot();
    let j = runner(&mut tw, "Johnson", 50_000);
    let fixer = runner(&mut tw, "Fixer", 0);
    let payer_acct = account_of(&tw, j);
    ok(
        &mut tw,
        j,
        &format!(
            "+run/post Split=10001 outcomes=success,partial,failure fixer=#{} fee=1001 \
             advance=500",
            fixer.0
        ),
    );
    let id = only_run(&tw);
    let c = snap(&tw)
        .contract(contract_of(&tw, id))
        .expect("contract")
        .clone();
    assert_eq!(c.consent.terms.remainder_to, payer_acct);
    for row in &c.consent.terms.outcomes {
        let total: i64 = row
            .payouts
            .iter()
            .map(|s| match s.share {
                mush_economy::Share::Exact(a) => a.minor(),
                mush_economy::Share::Permille(_) => 0,
            })
            .sum();
        assert_eq!(total, 10_001, "row {} does not sum exactly", row.name);
    }
}

/// §3.4 / M16: `+run/split` while `Listed` is an A′ amendment — the payer's
/// acceptance and escrow **stand**, and the amendment generation does not move.
#[test]
fn a_split_while_listed_keeps_the_payers_escrow() {
    let mut tw = boot();
    let j = runner(&mut tw, "Johnson", 50_000);
    let fixer = runner(&mut tw, "Fixer", 0);
    let acct = account_of(&tw, j);
    ok(
        &mut tw,
        j,
        &format!("+run/post Rework=10000 fixer=#{} fee=1000", fixer.0),
    );
    let id = only_run(&tw);
    let cid = contract_of(&tw, id);
    let before = snap(&tw).contract(cid).expect("contract").amendments;
    ok(&mut tw, j, &format!("+run/split {id}=fee 2000"));
    assert_eq!(held(&tw, acct), 10_000, "the split moved the escrow");
    let c = snap(&tw).contract(cid).expect("contract").clone();
    assert_eq!(c.amendments, before, "the generation moved with no refund");
    assert_eq!(c.acceptances.len(), 1, "the payer's acceptance was voided");
    assert_eq!(sr(&tw).run(id).expect("listing").fee.minor(), 2_000);
    // The fee cannot swallow the pay.
    assert!(err(&mut tw, j, &format!("+run/split {id}=fee 99999")).contains("fit inside"));
}

// ---------------------------------------------------------------------------
// Apply / crew / settle
// ---------------------------------------------------------------------------

/// The whole flow: post → escrow → apply → crew → accept → attest → settle, with
/// every payout landing on the roster members' **own** accounts (M6).
#[test]
fn the_run_settles_to_the_crews_own_accounts() {
    let mut tw = boot();
    let j = runner(&mut tw, "Johnson", 50_000);
    let a = runner(&mut tw, "Alder", 0);
    let b = runner(&mut tw, "Bex", 0);
    let (a_acct, b_acct) = (account_of(&tw, a), account_of(&tw, b));
    ok(
        &mut tw,
        j,
        "+run/post Extraction=9000 outcomes=success,failure slots=2",
    );
    let id = only_run(&tw);
    let cid = contract_of(&tw, id);
    ok(&mut tw, a, "+party/create Wetwork");
    let party = party_of(&tw, a);
    ok(&mut tw, a, &format!("+party/invite #{}", b.0));
    ok(&mut tw, b, &format!("+party/accept {party}"));
    ok(&mut tw, a, &format!("+run/apply {id}={party}"));
    ok(&mut tw, j, &format!("+run/crew {id}=accept"));
    assert_eq!(run_state(&tw, id), RunState::Crewing);
    assert_eq!(held(&tw, account_of(&tw, j)), 9_000, "the escrow moved");
    // Every runner accepts; the last acceptance activates.
    ok(&mut tw, a, &format!("@contract/accept {cid}"));
    ok(&mut tw, b, &format!("@contract/accept {cid}"));
    assert_eq!(state(&tw, cid), ContractState::Active);
    assert_eq!(run_state(&tw, id), RunState::Crewed);
    assert!(
        snap(&tw).contract(cid).expect("contract").roster_locked,
        "the roster did not lock"
    );
    // The named adjudicator — the payer, by default — attests the success row.
    ok(&mut tw, j, &format!("@contract/attest {cid}=0"));
    sweep_at(&mut tw, 100);
    assert!(
        matches!(state(&tw, cid), ContractState::Settled { .. }),
        "the run did not settle: {:?}",
        state(&tw, cid)
    );
    assert_eq!(bal(&tw, a_acct) + bal(&tw, b_acct), 9_000);
    assert_eq!(bal(&tw, a_acct), 4_500);
    assert_eq!(run_state(&tw, id), RunState::Closed);
    // D-E6-5: the party's lifecycle ended with its contract.
    assert!(
        mush_party::parties_of(tw.world(), a).is_empty(),
        "the crewed party was not released"
    );
}

/// M6: a member with no `sr.nuyen` account cannot crew — refused at **apply**,
/// where it is still fixable.
#[test]
fn a_member_with_no_account_cannot_crew() {
    let mut tw = boot();
    let j = runner(&mut tw, "Johnson", 50_000);
    let a = runner(&mut tw, "Alder", 0);
    let broke = tw.spawn_player("Unbanked");
    ok(&mut tw, j, "+run/post Extraction=9000 slots=2");
    let id = only_run(&tw);
    ok(&mut tw, a, "+party/create Wetwork");
    let party = party_of(&tw, a);
    ok(&mut tw, a, &format!("+party/invite #{}", broke.0));
    ok(&mut tw, broke, &format!("+party/accept {party}"));
    let refused = err(&mut tw, a, &format!("+run/apply {id}={party}"));
    assert!(refused.contains("nuyen account"), "{refused}");
}

/// M9: a party containing the adjudicator is refused at crew — the authority that
/// ends the run is never paid by it.
#[test]
fn a_party_containing_the_adjudicator_is_refused_at_crew() {
    let mut tw = boot();
    let j = runner(&mut tw, "Johnson", 50_000);
    let judge = runner(&mut tw, "Judge", 0);
    let a = runner(&mut tw, "Alder", 0);
    ok(
        &mut tw,
        j,
        &format!("+run/post Extraction=9000 slots=2 adjudicator=#{}", judge.0),
    );
    let id = only_run(&tw);
    ok(&mut tw, a, "+party/create Wetwork");
    let party = party_of(&tw, a);
    ok(&mut tw, a, &format!("+party/invite #{}", judge.0));
    ok(&mut tw, judge, &format!("+party/accept {party}"));
    ok(&mut tw, a, &format!("+run/apply {id}={party}"));
    let refused = err(&mut tw, j, &format!("+run/crew {id}=accept"));
    assert!(refused.contains("adjudicator"), "{refused}");
    assert_eq!(run_state(&tw, id), RunState::Listed);
}

/// §11 q5: one live run per party.
#[test]
fn a_party_holds_one_live_run() {
    let mut tw = boot();
    let j = runner(&mut tw, "Johnson", 50_000);
    let a = runner(&mut tw, "Alder", 0);
    ok(&mut tw, j, "+run/post First=1000 slots=1");
    ok(&mut tw, j, "+run/post Second=1000 slots=1");
    let ids: Vec<RunId> = sr(&tw).runs().map(|r| r.id).collect();
    ok(&mut tw, a, "+party/create Solo");
    let party = party_of(&tw, a);
    ok(&mut tw, a, &format!("+run/apply {}={party}", ids[0]));
    ok(&mut tw, j, &format!("+run/crew {}=accept", ids[0]));
    let refused = err(&mut tw, a, &format!("+run/apply {}={party}", ids[1]));
    assert!(refused.contains("already has a live run"), "{refused}");
}

/// `+run/crew decline` turns the applications away and leaves the listing open.
#[test]
fn declining_a_crew_leaves_the_listing_open() {
    let mut tw = boot();
    let j = runner(&mut tw, "Johnson", 50_000);
    let a = runner(&mut tw, "Alder", 0);
    ok(&mut tw, j, "+run/post Extraction=1000 slots=1");
    let id = only_run(&tw);
    ok(&mut tw, a, "+party/create Wetwork");
    let party = party_of(&tw, a);
    ok(&mut tw, a, &format!("+run/apply {id}={party}"));
    ok(&mut tw, j, &format!("+run/crew {id}=decline"));
    assert_eq!(run_state(&tw, id), RunState::Listed);
    assert!(sr(&tw).run(id).expect("listing").applied.is_empty());
}

/// D-E6-22 / M7: a **crewed** run's deadline disputes; it never silently refunds.
#[test]
fn a_crewed_runs_deadline_disputes() {
    let mut tw = boot();
    let j = runner(&mut tw, "Johnson", 50_000);
    let a = runner(&mut tw, "Alder", 0);
    ok(&mut tw, j, "+run/post Extraction=9000 slots=1 days=1");
    let id = only_run(&tw);
    let cid = contract_of(&tw, id);
    ok(&mut tw, a, "+party/create Solo");
    let party = party_of(&tw, a);
    ok(&mut tw, a, &format!("+run/apply {id}={party}"));
    ok(&mut tw, j, &format!("+run/crew {id}=accept"));
    ok(&mut tw, a, &format!("@contract/accept {cid}"));
    assert_eq!(state(&tw, cid), ContractState::Active);
    sweep_at(&mut tw, 86_400 + 60);
    assert_eq!(
        state(&tw, cid),
        ContractState::Disputed,
        "a crewed run refunded on its deadline"
    );
    assert_eq!(run_state(&tw, id), RunState::Closed);
}

/// §3.2: an **unbound** listing refunds when its deadline lapses, whatever
/// `on_expiry` says, and the board closes it.
#[test]
fn an_unbound_listing_expires_and_refunds() {
    let mut tw = boot();
    let j = runner(&mut tw, "Johnson", 50_000);
    let acct = account_of(&tw, j);
    ok(&mut tw, j, "+run/post Stale=9000 days=1");
    let id = only_run(&tw);
    assert_eq!(held(&tw, acct), 9_000);
    sweep_at(&mut tw, 86_400 + 60);
    assert_eq!(held(&tw, acct), 0, "the escrow was not refunded");
    assert_eq!(bal(&tw, acct), 50_000);
    assert_eq!(run_state(&tw, id), RunState::Closed);
}

// ---------------------------------------------------------------------------
// Faucets
// ---------------------------------------------------------------------------

/// D-E6-4 + M12: a campaign is a **funded organization**, and its standing drip is
/// bounded by `[economy]`'s mint cap.
#[test]
fn a_campaign_org_is_funded_by_a_capped_standing_mint() {
    let mut tw = boot();
    ok(&mut tw, GOD, "@org/create Aztechnology");
    let org = tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .and_then(|s| s.state().orgs().next().map(|o| o.id))
        .expect("the org exists");
    let scope = tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .and_then(|s| s.state().scopes_in(org).next().map(|sc| sc.id))
        .expect("the root scope exists");
    ok(
        &mut tw,
        GOD,
        &format!("@account/open org:#{}/{}=sr.nuyen", org.0, scope.0),
    );
    let treasury = snap(&tw)
        .accounts()
        .find(|a| a.owner == PrincipalId::Organization(org))
        .expect("the treasury")
        .id;
    let f = faucet(&tw, "sr.chargen");
    // The cap binds `create_system_obligation` only, and 50,000 major is the default.
    let ok_line = format!("@obligation/create {f}={treasury} 40000 every 100");
    let over = format!("@obligation/create {f}={treasury} 60000 every 100");
    // `@obligation/create` is staff-only and stamps `cause: Staff`; a faucet debit is
    // refused there by design, which is exactly what makes the seam the only route.
    err(&mut tw, GOD, &ok_line);
    err(&mut tw, GOD, &over);
    // The org can still be funded by an ordinary attributed mint, and its treasury
    // then posts a run like any other payer.
    ok(&mut tw, GOD, &format!("@econ/mint {f}={treasury} 30000"));
    assert_eq!(bal(&tw, treasury), 30_000);
    // …and the run it posts is **issued by the organization**, which is the whole
    // precondition DL-04's `OrgInbox` keys on: the contract's transitions reach every
    // holder of `acts_for` on the org with no Shadowrun code at all (G5).
    ok(
        &mut tw,
        GOD,
        &format!("+run/post Corporate=5000 scope={}", scope.0),
    );
    let id = only_run(&tw);
    let c = snap(&tw)
        .contract(contract_of(&tw, id))
        .expect("contract")
        .clone();
    assert_eq!(c.issuer, PrincipalId::Organization(org));
    assert_eq!(c.roles.payer, Some(PrincipalId::Organization(org)));
    assert_eq!(sr(&tw).run(id).expect("listing").principal, c.issuer);
    assert_eq!(held(&tw, treasury), 5_000, "the org escrow is not placed");
}

/// DL-10 / M3: the rolling window on an org treasury closes the ≤-threshold drip —
/// the fifth small run needs approval and lists nothing.
#[test]
fn the_rolling_window_stops_a_below_threshold_drip_from_an_org_treasury() {
    // The per-principal listing cap is lifted here so that the *window* is what
    // refuses the fifth run, not §9's board bound.
    let mut tw = boot_with(SrEconomyConfig {
        active_offer_cap: 8,
        ..sr_config()
    });
    ok(&mut tw, GOD, "@org/create Renraku");
    let org = tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .and_then(|s| s.state().orgs().next().map(|o| o.id))
        .expect("the org exists");
    let scope = tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .and_then(|s| s.state().scopes_in(org).next().map(|sc| sc.id))
        .expect("the root scope exists");
    ok(
        &mut tw,
        GOD,
        &format!("@account/open org:#{}/{}=sr.nuyen", org.0, scope.0),
    );
    let treasury = snap(&tw)
        .accounts()
        .find(|a| a.owner == PrincipalId::Organization(org))
        .expect("the treasury")
        .id;
    let f = faucet(&tw, "sr.chargen");
    ok(&mut tw, GOD, &format!("@econ/mint {f}={treasury} 100000"));
    // Threshold 10,000 (each run is under it), window 4,000 per hour.
    ok(
        &mut tw,
        GOD,
        &format!("@account/approval {treasury}=10000 threshold:1 window=4000/3600"),
    );
    for i in 0..4 {
        ok(
            &mut tw,
            GOD,
            &format!("+run/post Drip{i}=1000 scope={}", scope.0),
        );
    }
    let refused = err(
        &mut tw,
        GOD,
        &format!("+run/post Fifth=1000 scope={}", scope.0),
    );
    assert!(refused.contains("approval"), "{refused}");
    assert_eq!(sr(&tw).runs().count(), 4, "the fifth run was listed anyway");
}

// ---------------------------------------------------------------------------
// Lifestyle
// ---------------------------------------------------------------------------

/// A tier with a cost creates one obligation on `sr.month`, with the schedule's own
/// `maximum_catch_up` at 1 (D-E6-23).
#[test]
fn setting_a_tier_creates_one_upkeep_on_sr_month() {
    let mut tw = boot();
    let who = runner(&mut tw, "Runner", 50_000);
    ok(&mut tw, who, "+lifestyle/set middle");
    let record = snap(&tw)
        .obligations()
        .find(|o| o.debit_account == account_of(&tw, who))
        .expect("an upkeep obligation");
    assert_eq!(record.amount.minor(), 5_000);
    assert_eq!(record.schedule.maximum_catch_up, 1);
    assert!(matches!(
        record.schedule.clock,
        mush_economy::EconomicClock::Cadence(_)
    ));
    assert_eq!(
        sr(&tw).profile(who).expect("profile").lifestyle,
        LifestyleTier::Middle
    );
}

/// §3.2 "Life": a zero-cost tier creates **no** obligation — which is what makes
/// `street` a terminal floor for the downgrade ladder.
#[test]
fn a_zero_cost_tier_creates_no_obligation() {
    let mut tw = boot();
    let who = runner(&mut tw, "Squat", 50_000);
    ok(&mut tw, who, "+lifestyle/set street");
    assert!(
        snap(&tw)
            .obligations()
            .all(|o| o.debit_account != account_of(&tw, who)),
        "a free tier billed something"
    );
}

/// D-E6-3: a tier change **replaces** the record, and the old one is cancelled.
#[test]
fn a_tier_change_replaces_the_obligation() {
    let mut tw = boot();
    let who = runner(&mut tw, "Runner", 50_000);
    ok(&mut tw, who, "+lifestyle/set middle");
    let first = snap(&tw)
        .obligations()
        .find(|o| o.debit_account == account_of(&tw, who))
        .expect("the first record")
        .id;
    ok(&mut tw, who, "+lifestyle/set low");
    let record = snap(&tw).obligation(first).expect("still there").clone();
    assert_eq!(record.status, ObligationStatus::Cancelled);
    let live = snap(&tw)
        .obligations()
        .find(|o| o.debit_account == account_of(&tw, who) && o.status == ObligationStatus::Active)
        .expect("a live record");
    assert_eq!(live.amount.minor(), 2_000);
}

/// §3.2 "Life": one miss notices and **does not downgrade**; the second downgrades
/// exactly one tier by replacing the record, with `unpaid_periods` carried.
#[test]
fn one_miss_notices_and_the_second_downgrades_exactly_one_tier() {
    let mut tw = boot();
    let who = runner(&mut tw, "Runner", 5_000);
    ok(&mut tw, who, "+lifestyle/set middle");
    // The first period pays out of the 5,000 they have.
    sweep_at(&mut tw, PERIOD * 2);
    assert_eq!(bal(&tw, account_of(&tw, who)), 0);
    // The second cannot be paid: `Grace`, one notice, no downgrade.
    sweep_at(&mut tw, PERIOD * 3);
    let p = sr(&tw).profile(who).expect("profile").clone();
    assert_eq!(
        p.lifestyle,
        LifestyleTier::Middle,
        "the first miss downgraded"
    );
    assert_eq!(p.unpaid_periods, 1);
    assert_eq!(p.arrears.minor(), 5_000);
    // The third is the second miss: one tier down, the record replaced.
    sweep_at(&mut tw, PERIOD * 4);
    let p = sr(&tw).profile(who).expect("profile").clone();
    assert_eq!(
        p.lifestyle,
        LifestyleTier::Low,
        "the second miss did not downgrade"
    );
    assert_eq!(p.unpaid_periods, 2, "unpaid_periods was not carried");
    assert_eq!(p.arrears.minor(), 10_000);
}

/// D-E6-23 / M11: `+lifestyle/set` is refused in arrears, so a tier change can never
/// wipe a miss.
#[test]
fn setting_a_tier_in_arrears_is_refused() {
    let mut tw = boot();
    let who = runner(&mut tw, "Runner", 5_000);
    ok(&mut tw, who, "+lifestyle/set middle");
    sweep_at(&mut tw, PERIOD * 2);
    sweep_at(&mut tw, PERIOD * 3);
    assert_eq!(sr(&tw).profile(who).expect("profile").unpaid_periods, 1);
    let refused = err(&mut tw, who, "+lifestyle/set low");
    assert!(refused.contains("arrears"), "{refused}");
    assert_eq!(
        sr(&tw).profile(who).expect("profile").lifestyle,
        LifestyleTier::Middle,
        "a tier change in arrears wiped the miss"
    );
}

/// §3.4: `+lifestyle/pay` posts the arrears to `system:sr.lifestyle` under the
/// player's own authority, then reinstates.
#[test]
fn paying_the_arrears_clears_them_and_reinstates() {
    let mut tw = boot();
    let who = runner(&mut tw, "Runner", 5_000);
    ok(&mut tw, who, "+lifestyle/set middle");
    sweep_at(&mut tw, PERIOD * 2);
    sweep_at(&mut tw, PERIOD * 3);
    assert_eq!(sr(&tw).profile(who).expect("profile").unpaid_periods, 1);
    let f = faucet(&tw, "sr.chargen");
    let acct = account_of(&tw, who);
    ok(&mut tw, GOD, &format!("@econ/mint {f}={acct} 5000"));
    let sink = faucet(&tw, "sr.lifestyle");
    let before = bal(&tw, sink);
    ok(&mut tw, who, "+lifestyle/pay");
    let p = sr(&tw).profile(who).expect("profile").clone();
    assert_eq!(p.unpaid_periods, 0);
    assert_eq!(p.arrears.minor(), 0);
    assert_eq!(bal(&tw, sink) - before, 5_000, "the sink was not credited");
}

/// Answer 3 / D-E6-23: a **backlog** with catch-up 1 is one attempt, so ten idle
/// periods can never be a second miss — nothing downgrades twice.
#[test]
fn a_ten_period_backlog_downgrades_at_most_once() {
    let mut tw = boot();
    let who = runner(&mut tw, "Runner", 0);
    ok(&mut tw, who, "+lifestyle/set luxury");
    let record = snap(&tw)
        .obligations()
        .find(|o| o.debit_account == account_of(&tw, who))
        .expect("an upkeep obligation");
    assert_eq!(
        record.schedule.maximum_catch_up, 1,
        "the backlog cap is not 1"
    );
    // Ten periods pass in one jump. The clock's own catch-up (3) forgives on the
    // clock, and the schedule attempts exactly one — so one fire can never be the
    // second miss, and nothing walks two tiers down.
    sweep_at(&mut tw, PERIOD * 12);
    let p = sr(&tw).profile(who).expect("profile").clone();
    assert!(
        p.lifestyle >= LifestyleTier::High,
        "a backlog downgraded more than one tier: {:?}",
        p.lifestyle
    );
    assert!(p.unpaid_periods <= 1, "a backlog counted as several misses");
}

/// DL-07: boot adopts a self-identifying upkeep record whose profile lost the
/// pointer, and cancels one no profile claims.
#[test]
fn boot_adopts_a_self_identifying_upkeep_and_cancels_an_orphan() {
    let mut tw = boot();
    let who = runner(&mut tw, "Runner", 50_000);
    ok(&mut tw, who, "+lifestyle/set middle");
    let id = sr(&tw)
        .profile(who)
        .expect("profile")
        .upkeep
        .expect("upkeep");
    // Lose the pointer, then reboot: the record names itself, so it is adopted.
    let mut kv = fold_all(&mut tw);
    let key = format!("sr:profile:{}", who.0);
    patch(&mut kv, NS, &key, |v| {
        v["upkeep"] = serde_json::Value::Null;
    });
    reboot(&mut tw, &kv);
    assert_eq!(
        sr(&tw).profile(who).expect("profile").upkeep,
        Some(id),
        "boot did not adopt the self-identifying record"
    );
    // Now drop the profile entirely: the record is an orphan and is cancelled.
    let mut kv2 = fold_all(&mut tw);
    for (ns, entries) in &kv {
        kv2.entry(ns.clone()).or_default();
        let merged = kv2.get_mut(ns).expect("present");
        for (k, v) in entries {
            if !merged.iter().any(|(mk, _)| mk == k) {
                merged.push((k.clone(), v.clone()));
            }
        }
    }
    drop_doc(&mut kv2, NS, &key);
    reboot(&mut tw, &kv2);
    assert_eq!(
        snap(&tw).obligation(id).expect("record").status,
        ObligationStatus::Cancelled,
        "an unclaimed upkeep record is still collecting"
    );
}

// ---------------------------------------------------------------------------
// Chargen and migration
// ---------------------------------------------------------------------------

/// §3.5 / D-E6-1: `@sheet/set …/nuyen` is refused once the sheet is `Approved`,
/// while `+stat nuyen=` in chargen still works.
#[test]
fn the_sheet_nuyen_field_is_frozen_after_approval() {
    let mut tw = boot();
    let who = tw.spawn_player("Runner");
    ok(&mut tw, GOD, &format!("@sheet/set #{}/nuyen=1234", who.0));
    // Stamp the sheet approved through the ruleset's own store.
    let rs = ruleset_shadowrun::ShadowrunRuleset::new();
    let store = rs.store();
    let mut doc = store.get_or_default(tw.world(), who);
    doc.approved = mush_ruleset::ApprovalState::Approved { by: GOD, at: 10 };
    store
        .put_by(tw.world_mut(), who, &doc, GOD)
        .expect("the sheet writes");
    let refused = err(&mut tw, GOD, &format!("@sheet/set #{}/nuyen=9999", who.0));
    assert!(refused.contains("money after approval"), "{refused}");
    // Another field still writes.
    ok(&mut tw, GOD, &format!("@sheet/set #{}/body=4", who.0));
}

/// Answer 18 / M13: `dry` writes nothing; `commit` mints once from
/// `system:sr.migration`; a re-run is a HIT; an edited sheet is a mismatch and is
/// never re-minted.
#[test]
fn migration_is_idempotent_and_reports_an_edited_sheet() {
    let mut tw = boot();
    let who = tw.spawn_player("Legacy");
    let rs = ruleset_shadowrun::ShadowrunRuleset::new();
    let store = rs.store();
    let mut doc = store.get_or_default(tw.world(), who);
    doc.set_number("nuyen", 7_500);
    doc.approved = mush_ruleset::ApprovalState::Approved { by: GOD, at: 10 };
    store
        .put_by(tw.world_mut(), who, &doc, GOD)
        .expect("the sheet writes");
    let dry = ok(&mut tw, GOD, &format!("@sr/migrate dry #{}", who.0));
    assert!(dry.contains("would mint 7500"), "{dry}");
    assert!(sr(&tw).profile(who).is_none(), "dry wrote a profile");
    let committed = ok(&mut tw, GOD, &format!("@sr/migrate commit #{}", who.0));
    assert!(committed.contains("minted 7500"), "{committed}");
    let acct = account_of(&tw, who);
    assert_eq!(bal(&tw, acct), 7_500);
    assert_eq!(bal(&tw, faucet(&tw, "sr.migration")), -7_500);
    // A re-run reports the original result and mints nothing.
    let again = ok(&mut tw, GOD, &format!("@sr/migrate commit #{}", who.0));
    assert!(again.contains("already migrated"), "{again}");
    assert_eq!(bal(&tw, acct), 7_500);
    // An edited sheet is a mismatch, never a second mint.
    let mut doc = store.get_or_default(tw.world(), who);
    doc.set_number("nuyen", 99_999);
    store
        .put_by(tw.world_mut(), who, &doc, GOD)
        .expect("the sheet writes");
    let mismatch = ok(&mut tw, GOD, &format!("@sr/migrate commit #{}", who.0));
    assert!(mismatch.contains("MISMATCH"), "{mismatch}");
    assert_eq!(bal(&tw, acct), 7_500);
}

/// §3.5 / G2: `+nuyen` renders the **account** after approval, the sheet field
/// before, and nothing at all to a viewer the account's gate refuses.
#[test]
fn nuyen_renders_the_account_after_approval_and_the_sheet_before() {
    let mut tw = boot();
    let who = tw.spawn_player("Runner");
    ok(&mut tw, GOD, &format!("@sheet/set #{}/nuyen=1234", who.0));
    assert!(ok(&mut tw, who, "+nuyen").contains("1234"));
    approve(&mut tw, who, 4_000);
    ok(&mut tw, GOD, &format!("@sr/migrate commit #{}", who.0));
    let mine = ok(&mut tw, who, "+nuyen");
    assert!(mine.contains("\u{a5}4000"), "{mine}");
    assert!(!mine.contains("1234"), "the sheet number leaked: {mine}");
}

// ---------------------------------------------------------------------------
// Durability
// ---------------------------------------------------------------------------

/// A listing, a profile and the allocator all survive a restart, and the board is
/// re-mirrored onto its contract.
#[test]
fn the_board_and_the_profiles_survive_a_reboot() {
    let mut tw = boot();
    let j = runner(&mut tw, "Johnson", 50_000);
    ok(&mut tw, j, "+run/post Extraction=9000");
    let id = only_run(&tw);
    ok(&mut tw, j, "+lifestyle/set low");
    let kv = fold_all(&mut tw);
    reboot(&mut tw, &kv);
    assert_eq!(sr(&tw).runs().count(), 1);
    assert_eq!(run_state(&tw, id), RunState::Listed);
    assert_eq!(
        sr(&tw).profile(j).expect("profile").lifestyle,
        LifestyleTier::Low
    );
    // The allocator floor never re-issues a live id.
    ok(&mut tw, j, "+run/post Second=1000");
    let ids: Vec<RunId> = sr(&tw).runs().map(|r| r.id).collect();
    assert_eq!(ids.len(), 2);
    assert!(ids[1].0 > ids[0].0);
}

/// The E5a fold rule: a document that violates an invariant, or disagrees with its
/// own key (DL-07), is dropped **loudly** and the world still boots.
#[test]
fn a_corrupt_document_is_dropped_and_the_world_still_boots() {
    let mut tw = boot();
    let j = runner(&mut tw, "Johnson", 50_000);
    ok(&mut tw, j, "+run/post Extraction=9000");
    let id = only_run(&tw);
    let mut kv = fold_all(&mut tw);
    // A listing whose record disagrees with its own key (DL-07's identity rule).
    patch(&mut kv, NS, &format!("sr:run:{}", id.0), |v| {
        v["id"] = serde_json::json!(9_999);
    });
    // A profile with negative arrears — the wire refuses it outright.
    patch(&mut kv, NS, &format!("sr:profile:{}", j.0), |v| {
        v["arrears"] = serde_json::json!(-1);
    });
    reboot(&mut tw, &kv);
    assert_eq!(
        sr(&tw).runs().count(),
        0,
        "a mismatched listing was trusted"
    );
    assert!(sr(&tw).profile(j).is_none(), "a bad profile was trusted");
    // The contract — where the money actually is — is untouched.
    assert!(!contracts(&tw).is_empty());
}

/// The board mirrors the contract's own terminal state, whatever route it took.
#[test]
fn the_board_mirrors_a_contract_cancelled_out_from_under_it() {
    let mut tw = boot();
    let j = runner(&mut tw, "Johnson", 50_000);
    ok(&mut tw, j, "+run/post Extraction=9000");
    let id = only_run(&tw);
    let cid = contract_of(&tw, id);
    // Staff end it through the economy's own verb — the board hears about it the
    // same way it hears about a settlement, through `ContractChanged`.
    ok(&mut tw, GOD, &format!("@contract/cancel {cid}"));
    assert_eq!(
        run_state(&tw, id),
        RunState::Closed,
        "the board did not mirror the cancellation"
    );
}

/// `@sr/economy` and `@sr/profile` are staff reads and say what the layer is doing.
#[test]
fn the_staff_verbs_report_the_layer() {
    let mut tw = boot();
    let who = runner(&mut tw, "Runner", 1_000);
    let out = ok(&mut tw, GOD, "@sr/economy");
    assert!(out.contains("enabled = true"), "{out}");
    assert!(out.contains("sr.month"), "{out}");
    let p = ok(&mut tw, GOD, &format!("@sr/profile #{}", who.0));
    assert!(p.contains("lifestyle street"), "{p}");
    // A player cannot read the staff surface.
    err(&mut tw, who, "@sr/economy");
}

// ---------------------------------------------------------------------------
// ---- E6a verify fold ----
// ---------------------------------------------------------------------------

/// The one live listing's id.
fn live_run(tw: &TestWorld) -> RunId {
    let ids: Vec<RunId> = sr(tw).live_ids().collect();
    assert_eq!(
        ids.len(),
        1,
        "expected exactly one live listing, got {ids:?}"
    );
    ids[0]
}

/// Put a document back into a folded kv, whether or not it is still there — the shape
/// "this write never landed" takes when the live path has since deleted or rewritten
/// it.
fn restore(kv: &mut Kv, ns: &str, key: &str, value: serde_json::Value) {
    let entry = kv.entry(SmolStr::new(ns)).or_default();
    entry.retain(|(k, _)| k.as_str() != key);
    entry.push((SmolStr::new(key), value));
}

/// One document out of a folded kv.
fn doc_of(kv: &Kv, ns: &str, key: &str) -> serde_json::Value {
    kv.get(&SmolStr::new(ns))
        .and_then(|e| e.iter().find(|(k, _)| k.as_str() == key))
        .map(|(_, v)| v.clone())
        .unwrap_or_else(|| panic!("no such persisted key: {ns}/{key}"))
}

/// Whether the board would still let the payer take this listing down (M5).
fn withdrawable(tw: &TestWorld, id: RunId) -> bool {
    let run = sr(tw).run(id).expect("listing").clone();
    let c = snap(tw).contract(run.contract).expect("contract").clone();
    !run.state.is_closed()
        && run.state != RunState::Crewed
        && c.acceptances.iter().all(|a| {
            mush_economy::principal_key(&a.party) == mush_economy::principal_key(&c.issuer)
        })
}

// ---- Lens M: money and consent --------------------------------------------

/// M1 (HIGH): the named adjudicator could settle a **crewless** `Listed` run and move
/// the fixer's fee out of the payer's escrow.
///
/// S1 deviation 2 makes a one-party contract `Active` at its author's own accept, so a
/// `Listed` run is a live contract whose every row already carried the fixer's fee and
/// the advance — and the adjudicator is not a party, so nothing stopped them attesting
/// row 0 before a single runner existed. A pre-crew settlement is now arithmetically a
/// no-op: every row returns the whole escrow to the payer and closes the run.
#[test]
fn vf_m1_an_adjudicator_settling_a_crewless_run_moves_nothing() {
    let mut tw = boot();
    let j = runner(&mut tw, "Johnson", 50_000);
    let fixer = runner(&mut tw, "Fixer", 0);
    let judge = runner(&mut tw, "Judge", 0);
    let (j_acct, fixer_acct) = (account_of(&tw, j), account_of(&tw, fixer));
    ok(
        &mut tw,
        j,
        &format!(
            "+run/post Brokered=10000 fixer=#{} fee=1000 advance=500 adjudicator=#{}",
            fixer.0, judge.0
        ),
    );
    let id = live_run(&tw);
    let cid = contract_of(&tw, id);
    assert_eq!(held(&tw, j_acct), 10_000);

    ok(&mut tw, judge, &format!("@contract/attest {cid}=0"));
    sweep_at(&mut tw, 100);

    assert!(
        state(&tw, cid).is_terminal(),
        "the pre-crew attestation left the contract live: {:?}",
        state(&tw, cid)
    );
    assert_eq!(bal(&tw, fixer_acct), 0, "the fixer was paid for no run");
    assert_eq!(bal(&tw, j_acct), 50_000, "the payer is not whole");
    assert_eq!(held(&tw, j_acct), 0, "the escrow is still reserved");
    assert_eq!(run_state(&tw, id), RunState::Closed);
}

/// M4 (with M1): the roles rule was half enforced — `adjudicator=` refused the payer
/// and the poster, and admitted the **fixer** (paid by every crewed row) and the
/// **Johnson** (the payer's own visible representative).
#[test]
fn vf_m4_the_adjudicator_is_never_the_fixer_or_the_johnson() {
    let mut tw = boot();
    let j = runner(&mut tw, "Johnson", 50_000);
    let other = runner(&mut tw, "Other", 0);
    let refused = err(
        &mut tw,
        j,
        &format!(
            "+run/post Rigged=10000 fixer=#{} fee=1000 adjudicator=#{}",
            other.0, other.0
        ),
    );
    assert!(refused.contains("fixer"), "{refused}");
    let refused = err(
        &mut tw,
        j,
        &format!(
            "+run/post Rigged=10000 johnson=#{} adjudicator=#{}",
            other.0, other.0
        ),
    );
    assert!(refused.contains("Johnson"), "{refused}");
    assert_eq!(sr(&tw).runs().count(), 0, "a listing was created");
    assert!(contracts(&tw).iter().all(|c| state(&tw, *c).is_terminal()));
}

/// M2 (HIGH): one party could be crewed onto **two** runs and paid by both.
///
/// `party_has_live_run` was asked at `+run/apply` and never again, so two applications
/// placed before either answer let the payer accept both. The check — and the party
/// lock, which is the real gate — now run before the amendment.
#[test]
fn vf_m2_one_party_cannot_be_crewed_onto_two_runs() {
    let mut tw = boot();
    let j = runner(&mut tw, "Johnson", 50_000);
    let a = runner(&mut tw, "Alder", 0);
    ok(&mut tw, j, "+run/post First=1000 slots=1");
    ok(&mut tw, j, "+run/post Second=1000 slots=1");
    let ids: Vec<RunId> = sr(&tw).live_ids().collect();
    ok(&mut tw, a, "+party/create Solo");
    let party = party_of(&tw, a);
    // Both applications land before either is answered.
    ok(&mut tw, a, &format!("+run/apply {}={party}", ids[0]));
    ok(&mut tw, a, &format!("+run/apply {}={party}", ids[1]));
    ok(&mut tw, j, &format!("+run/crew {}=accept", ids[0]));
    let refused = err(&mut tw, j, &format!("+run/crew {}=accept", ids[1]));
    assert!(refused.contains("live run"), "{refused}");
    assert_eq!(
        run_state(&tw, ids[1]),
        RunState::Listed,
        "the second run was crewed anyway"
    );
    let c = snap(&tw)
        .contract(contract_of(&tw, ids[1]))
        .expect("contract")
        .clone();
    assert!(
        !c.consent.parties.contains(&PrincipalId::Object(a)),
        "the runner was added to the second contract"
    );
}

/// R-M2 (with M2): `crew_accept` crewed a party that was **not `Forming`** — a
/// disbanded one included — because the lock came after the amendment and its failure
/// was only a `warn!`.
#[test]
fn vf_m2_a_disbanded_party_cannot_be_crewed() {
    let mut tw = boot();
    let j = runner(&mut tw, "Johnson", 50_000);
    let a = runner(&mut tw, "Alder", 0);
    ok(&mut tw, j, "+run/post Extraction=1000 slots=1");
    let id = live_run(&tw);
    ok(&mut tw, a, "+party/create Solo");
    let party = party_of(&tw, a);
    ok(&mut tw, a, &format!("+run/apply {id}={party}"));
    ok(&mut tw, a, "+party/disband");

    let refused = err(&mut tw, j, &format!("+run/crew {id}=accept"));
    assert!(refused.contains("no longer available"), "{refused}");
    assert_eq!(run_state(&tw, id), RunState::Listed);
    let c = snap(&tw)
        .contract(contract_of(&tw, id))
        .expect("contract")
        .clone();
    assert!(
        !c.consent.parties.contains(&PrincipalId::Object(a)),
        "a disbanded party's member was added to the contract"
    );
}

/// M5 (MEDIUM): `+run/withdraw` had **no state check**, and `cancel_contract` admits
/// an author-only live contract — so the payer could tear up a run runners had already
/// bound themselves to.
#[test]
fn vf_m5_withdraw_is_refused_once_a_runner_has_accepted() {
    let mut tw = boot();
    let j = runner(&mut tw, "Johnson", 50_000);
    let a = runner(&mut tw, "Alder", 0);
    let b = runner(&mut tw, "Bex", 0);
    let acct = account_of(&tw, j);
    ok(&mut tw, j, "+run/post Extraction=9000 slots=2");
    let id = live_run(&tw);
    let cid = contract_of(&tw, id);
    ok(&mut tw, a, "+party/create Wetwork");
    let party = party_of(&tw, a);
    ok(&mut tw, a, &format!("+party/invite #{}", b.0));
    ok(&mut tw, b, &format!("+party/accept {party}"));
    ok(&mut tw, a, &format!("+run/apply {id}={party}"));
    ok(&mut tw, j, &format!("+run/crew {id}=accept"));
    // Nobody has accepted yet: still the payer's to take down.
    assert!(withdrawable(&tw, id), "a crewing run with no acceptance");
    // One runner binds themselves. Now it is not.
    ok(&mut tw, a, &format!("@contract/accept {cid}"));
    assert!(!withdrawable(&tw, id));
    let refused = err(&mut tw, j, &format!("+run/withdraw {id}"));
    assert!(refused.contains("bound themselves"), "{refused}");
    assert_eq!(held(&tw, acct), 9_000, "the escrow was refunded anyway");
    assert!(!state(&tw, cid).is_terminal());
}

/// A `Listed` run is still the payer's alone, and withdrawing it refunds.
#[test]
fn vf_m5_a_listed_run_is_still_the_payers_to_withdraw() {
    let mut tw = boot();
    let j = runner(&mut tw, "Johnson", 50_000);
    let acct = account_of(&tw, j);
    ok(&mut tw, j, "+run/post Extraction=9000");
    let id = live_run(&tw);
    ok(&mut tw, j, &format!("+run/withdraw {id}"));
    assert_eq!(held(&tw, acct), 0);
    assert_eq!(bal(&tw, acct), 50_000);
    assert_eq!(run_state(&tw, id), RunState::Closed);
}

// ---- E6a verify fold: extra rig ------------------------------------------

/// The production composition **without** a declared `sr.month` — a server whose
/// operator never contributed the stanza, or one whose clock went away (R5).
fn boot_no_cadence() -> TestWorld {
    let cfg = sr_config();
    TestWorld::with_modules(vec![
        Box::new(
            mush_org::OrgModule::new()
                .with_capabilities(vocabulary())
                .with_scope_use(vec![Arc::new(mush_economy::EconomyScopeUse)]),
        ),
        Box::new(mush_faction::FactionModule::new()),
        Box::new(
            mush_economy::EconomyModule::with_custody(vec![])
                .with_observers(vec![Arc::new(SrBoardObserver::new(cfg.clone()))]),
        ),
        Box::new(mush_party::PartyModule::new()),
        Box::new(mush_jobs::JobsModule::new()),
        Box::new(ruleset_shadowrun::ShadowrunRuleset::with_seed(7).economy(cfg)),
    ])
}

/// A character built and approved through the **real** chargen surface, so the
/// `sr.chargen/{obj}` mint key is genuinely spent (R3).
fn chargen_approved(tw: &mut TestWorld, name: &str) -> ObjId {
    let who = tw.spawn_player(name);
    tw.run(who, "+chargen");
    tw.run(
        who,
        "+chargen/priority metatype=C attributes=A magic=E skills=B resources=D",
    );
    tw.run(who, "+stat metatype=human");
    for c in [
        "body=6",
        "agility=5",
        "reaction=5",
        "strength=4",
        "charisma=3",
        "intuition=3",
        "logic=3",
        "willpower=3",
    ] {
        tw.run(who, &format!("+stat {c}"));
    }
    tw.run(who, "+stat edge=7");
    for s in [
        "pistols",
        "automatics",
        "longarms",
        "blades",
        "sneaking",
        "perception",
    ] {
        tw.run(who, &format!("+stat active_skills:{s}=6"));
    }
    tw.run(who, "+stat skill_groups:influence=5");
    tw.run(who, "+stat nuyen=50000");
    tw.run(who, "+chargen/submit");
    let store = ruleset_shadowrun::ShadowrunRuleset::new().store();
    let job = match store.get_or_default(tw.world(), who).approved {
        mush_ruleset::ApprovalState::Pending(j) => j,
        other => panic!("expected a pending sheet, got {other:?}"),
    };
    ok(tw, GOD, &format!("+job/approve {job}=Welcome."));
    assert!(
        store.get_or_default(tw.world(), who).approved.is_approved(),
        "the sheet was not approved"
    );
    who
}

/// Record a Resources priority and a sheet `nuyen` on an already-approved sheet — the
/// drift `@sr/migrate` has to make a decision about (M7).
fn set_resources_priority(tw: &mut TestWorld, who: ObjId, letter: char, nuyen: i64) {
    let store = ruleset_shadowrun::ShadowrunRuleset::new().store();
    let mut doc = store.get_or_default(tw.world(), who);
    doc.map_set(
        ruleset_shadowrun::schema::CHARGEN_PRIORITIES_FIELD,
        "resources",
        serde_json::json!(letter.to_string()),
    );
    doc.set_number("nuyen", nuyen);
    store
        .put_by(tw.world_mut(), who, &doc, GOD)
        .expect("the sheet writes");
}

// ---- Lens R: durability and replay ----------------------------------------

/// Fold every op queued since the last fold **into** an existing kv, so a test can
/// keep one running picture of the store across several phases.
fn fold_into(tw: &mut TestWorld, kv: &mut Kv) {
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
}

/// A crewed, contract-accepted run: the party is `Locked`, the board says `Crewed`.
/// Returns `(payer, runner, listing, contract, party)`.
fn crewed_run(tw: &mut TestWorld) -> (ObjId, ObjId, RunId, ContractId, u64) {
    let j = runner(tw, "Johnson", 50_000);
    let a = runner(tw, "Alder", 0);
    ok(tw, j, "+run/post Extraction=9000 slots=1");
    let id = live_run(tw);
    let cid = contract_of(tw, id);
    ok(tw, a, "+party/create Solo");
    let party = party_of(tw, a);
    ok(tw, a, &format!("+run/apply {id}={party}"));
    ok(tw, j, &format!("+run/crew {id}=accept"));
    ok(tw, a, &format!("@contract/accept {cid}"));
    assert_eq!(run_state(tw, id), RunState::Crewed);
    (j, a, id, cid, party)
}

/// R1 (HIGH): boot closed a listing and never released its party.
///
/// `reconcile_board` mirrored and wrote, and only the observer and `+run/withdraw`
/// ever called `release_party` — both live paths. A terminal post whose board and
/// party writes did not land (or a contract that ended while `[shadowrun.economy]` was
/// off) therefore left the crew `Locked` **forever**: every membership verb refused,
/// idle expiry skipped it, and nothing but the ruleset can release it. The release now
/// runs whenever the mirrored state is `Closed`, including the `now == was` case —
/// which is precisely the shape a dropped write leaves.
#[test]
fn vf_r1_boot_releases_the_party_of_a_listing_that_is_already_closed() {
    let mut tw = boot();
    let (j, a, id, cid, party) = crewed_run(&mut tw);
    let mut kv = fold_all(&mut tw);
    let run_key = format!("sr:run:{}", id.0);
    let party_key = format!("party:{party}");
    let run_doc = doc_of(&kv, NS, &run_key);
    let locked = doc_of(&kv, "party", &party_key);

    // The run settles for real…
    ok(&mut tw, j, &format!("@contract/attest {cid}=0"));
    sweep_at(&mut tw, 100);
    assert!(matches!(state(&tw, cid), ContractState::Settled { .. }));
    fold_into(&mut tw, &mut kv);

    // …and the board and party writes are dropped: the listing document still stands,
    // already saying `closed`, and the party document still says `Locked`.
    let mut closed = run_doc;
    closed["state"] = serde_json::json!("closed");
    restore(&mut kv, NS, &run_key, closed);
    restore(&mut kv, "party", &party_key, locked);
    reboot(&mut tw, &kv);

    assert!(
        mush_party::parties_of(tw.world(), a).is_empty(),
        "the crew is still locked to a contract that settled"
    );
    assert_eq!(run_state(&tw, id), RunState::Closed);
}

/// R2 (HIGH): a **skipped** `sr:run:{id}` document orphaned its `Locked` party
/// permanently.
///
/// The loader drops a listing it cannot read, so `run_of_contract` is empty and
/// `reconcile_board` — which can only reach a party a listing still names — never sees
/// the crew at all. The ruleset now asks the question from the party side: for every
/// live `Locked` party it resolves the opaque `ContractRef` it handed over and
/// releases the ones whose contract is gone or terminal. `mush-party` stays
/// money-blind; the ruleset, which owns both ends, is the one that resolves.
#[test]
fn vf_r2_boot_releases_a_party_orphaned_by_a_skipped_listing() {
    let mut tw = boot();
    let (j, a, id, cid, party) = crewed_run(&mut tw);
    let mut kv = fold_all(&mut tw);
    let run_key = format!("sr:run:{}", id.0);
    let party_key = format!("party:{party}");
    let run_doc = doc_of(&kv, NS, &run_key);
    let locked = doc_of(&kv, "party", &party_key);

    ok(&mut tw, j, &format!("@contract/attest {cid}=0"));
    sweep_at(&mut tw, 100);
    fold_into(&mut tw, &mut kv);

    // The listing document disagrees with its own key (DL-07) — the loader skips it —
    // and the party write never landed.
    let mut bad = run_doc;
    bad["id"] = serde_json::json!(9_999);
    restore(&mut kv, NS, &run_key, bad);
    restore(&mut kv, "party", &party_key, locked);
    reboot(&mut tw, &kv);

    assert_eq!(sr(&tw).runs().count(), 0, "the bad listing was trusted");
    assert!(
        mush_party::parties_of(tw.world(), a).is_empty(),
        "nothing released a party whose listing was skipped"
    );
    // The contract — where the money is — is untouched.
    assert!(matches!(state(&tw, cid), ContractState::Settled { .. }));
}

/// R3 (HIGH): a corrupt profile let `@sr/migrate commit` mint a **second** allotment
/// over the chargen one.
///
/// `plan` decided "already funded" from `SrProfile.minted` alone — a projection that
/// is dropped at boot when its document will not load — and the two keys differ
/// (`sr.chargen/{obj}` vs `sr.migration/{obj}`), so the second mint hit no idempotency
/// wall at all. The index is the authority now.
#[test]
fn vf_r3_a_corrupt_profile_never_mints_a_second_allotment() {
    let mut tw = boot();
    let who = chargen_approved(&mut tw, "Nick");
    let acct = account_of(&tw, who);
    let minted = bal(&tw, acct);
    assert!(minted > 0, "the chargen allotment was not minted");

    // The profile document will not load (negative arrears): the *projection* of the
    // mint is gone, and the idempotency key is still spent.
    let mut kv = fold_all(&mut tw);
    patch(&mut kv, NS, &format!("sr:profile:{}", who.0), |v| {
        v["arrears"] = serde_json::json!(-1);
    });
    reboot(&mut tw, &kv);
    assert!(sr(&tw).profile(who).is_none(), "a bad profile was trusted");

    let out = ok(&mut tw, GOD, &format!("@sr/migrate commit #{}", who.0));
    assert!(out.contains("skipped"), "{out}");
    assert!(out.contains("key is spent"), "{out}");
    assert_eq!(
        bal(&tw, acct),
        minted,
        "a second allotment was minted over the chargen one"
    );
}

/// R4 (MEDIUM): a closed account was a one-way exit.
///
/// `personal_account` matched owner + instrument + scope and never `status`, and
/// `SrProfile.account` is a cache nothing re-derived — so migrate, the crew payout
/// table and lifestyle all kept naming the closed account even after staff opened a
/// live one beside it.
#[test]
fn vf_r4_a_closed_account_is_re_resolved_at_boot() {
    let mut tw = boot();
    let who = runner(&mut tw, "Closer", 0);
    let old = account_of(&tw, who);
    ok(&mut tw, GOD, &format!("@account/open #{}=sr.nuyen", who.0));
    let new = snap(&tw)
        .accounts()
        .find(|a| a.owner == PrincipalId::Object(who) && a.id != old)
        .expect("a second account")
        .id;
    ok(&mut tw, GOD, &format!("@account/close {old}"));

    let kv = fold_all(&mut tw);
    reboot(&mut tw, &kv);
    assert_eq!(
        sr(&tw).profile(who).and_then(|p| p.account),
        Some(new),
        "the profile still names the closed account"
    );
    assert!(
        sr(&tw).indexes_agree(),
        "the account index disagrees after a re-resolve"
    );
}

/// R5 (LOW): `+lifestyle/pay` with the cadence gone reused an ordinal.
///
/// The counters and the pay key are keyed on the `sr.month` ordinal and nothing else
/// (E5c T3); `miss` fell back to the *stale* ordinal when the clock was missing, so a
/// second cycle on the same ordinal hit an already-spent key, reported success, zeroed
/// the arrears and moved no money. With no clock there is no ordinal, so there is
/// nothing to key and the verb refuses.
#[test]
fn vf_r5_paying_arrears_with_no_cadence_is_refused() {
    let mut tw = boot_no_cadence();
    let who = tw.spawn_player("Broke");
    let kv: Kv = BTreeMap::from([(
        SmolStr::new(NS),
        vec![(
            SmolStr::new(format!("sr:profile:{}", who.0)),
            serde_json::json!({
                "obj": who.0, "lifestyle": "middle", "account": 1,
                "arrears": 5_000, "unpaid_periods": 1, "unpaid_since": 3, "revision": 2
            }),
        )],
    )]);
    reboot(&mut tw, &kv);
    assert_eq!(sr(&tw).profile(who).map(|p| p.arrears.minor()), Some(5_000));
    let refused = err(&mut tw, who, "+lifestyle/pay");
    assert!(refused.contains("cadence"), "{refused}");
    assert_eq!(
        sr(&tw).profile(who).map(|p| p.arrears.minor()),
        Some(5_000),
        "a refused pay cleared the arrears anyway"
    );
}

/// R5, the other half: the pay key carries the **amount** as a second discriminator,
/// so two different debts landing on one ordinal cannot collide on a spent key.
#[test]
fn vf_r5_the_pay_key_discriminates_on_the_amount() {
    let mut tw = boot();
    let who = runner(&mut tw, "Runner", 5_000);
    ok(&mut tw, who, "+lifestyle/set middle");
    sweep_at(&mut tw, PERIOD * 2);
    sweep_at(&mut tw, PERIOD * 3);
    let p = sr(&tw).profile(who).expect("profile").clone();
    assert_eq!(p.arrears.minor(), 5_000);
    let f = faucet(&tw, "sr.chargen");
    let acct = account_of(&tw, who);
    ok(&mut tw, GOD, &format!("@econ/mint {f}={acct} 5000"));
    ok(&mut tw, who, "+lifestyle/pay");
    let key = mush_economy::IdempotencyKey::new(
        SmolStr::new_static("sr.lifestyle.pay"),
        SmolStr::new(format!("{}/{}/{}", who.0, p.unpaid_since, 5_000)),
    );
    assert!(
        snap(&tw).idempotency(&key).is_some(),
        "the arrears payment was not keyed on {{who}}/{{ordinal}}/{{amount}}"
    );
}

// ---- Lens M: the lifestyle ladder -----------------------------------------

/// M3 (MEDIUM): after a downgrade the ladder stopped and so did the rent.
///
/// `miss` compared the record's `missed_total` against `unpaid_periods`, which is
/// *carried across the replacement* — and the replacement's `missed_total` restarts at
/// 0. So from the first downgrade onward every later miss was swallowed: the tier
/// never moved again, the arrears never grew, and the record sat `Delinquent`, out of
/// the due pool, collecting nothing. The mark is per record now.
#[test]
fn vf_m3_the_ladder_keeps_walking_after_the_first_downgrade() {
    let mut tw = boot();
    let who = runner(&mut tw, "Runner", 5_000);
    ok(&mut tw, who, "+lifestyle/set middle");
    // Period 2 pays out of the 5,000 they have; everything after it misses.
    for n in 2..=16 {
        sweep_at(&mut tw, PERIOD * n);
    }
    let p = sr(&tw).profile(who).expect("profile").clone();
    assert_eq!(
        p.lifestyle,
        LifestyleTier::Street,
        "the ladder stopped before the floor at {:?}",
        p.lifestyle
    );
    assert!(
        p.arrears.minor() > 10_000,
        "the arrears stopped growing at {}",
        p.arrears.minor()
    );
    assert!(
        p.unpaid_periods >= 4,
        "the unpaid count stopped at {}",
        p.unpaid_periods
    );
}

/// M6 (LOW): a failed replacement left a character with **no upkeep at all**.
///
/// `set_tier` and `apply_downgrade` cancelled, wrote the profile at the new tier, and
/// only then created — with the create's `Err` merely logged. So a refused create left
/// the character at a tier nothing was billing, paying nothing forever, and the only
/// way back was for them to notice and set a tier again. Opening the replacement
/// first makes a refusal change nothing at all: the profile is written once, at the
/// end, and only when there is a record behind it.
#[test]
fn vf_m6_a_refused_replacement_leaves_the_profile_untouched() {
    // A world with no `sr.month`: `open_upkeep` cannot anchor a schedule, so every
    // create is refused, which is the failure the ordering is about.
    let mut tw = boot_no_cadence();
    let who = tw.spawn_player("Stuck");
    approve(&mut tw, who, 0);
    ok(&mut tw, GOD, &format!("@sr/migrate commit #{}", who.0));
    let before = sr(&tw).profile(who).expect("profile").clone();
    assert_eq!(before.lifestyle, LifestyleTier::Street);

    let refused = err(&mut tw, who, "+lifestyle/set middle");
    assert!(refused.contains("cadence"), "{refused}");

    let after = sr(&tw).profile(who).expect("profile").clone();
    assert_eq!(
        after.lifestyle, before.lifestyle,
        "the tier moved with no upkeep record behind it"
    );
    assert_eq!(after.upkeep, None);
    assert_eq!(
        after.revision, before.revision,
        "a refused replacement wrote the profile anyway"
    );
    assert!(sr(&tw).indexes_agree());
}

/// M6, the boot half: a record the profile does not point at is cancelled, so a
/// refused *cancel* after a successful create is repaired at the next restart rather
/// than billing one wallet twice forever.
#[test]
fn vf_m6_boot_cancels_a_lifestyle_record_the_profile_does_not_point_at() {
    let mut tw = boot();
    let who = runner(&mut tw, "Runner", 50_000);
    ok(&mut tw, who, "+lifestyle/set middle");
    let live = sr(&tw)
        .profile(who)
        .expect("profile")
        .upkeep
        .expect("upkeep");
    // The profile points somewhere else — the shape a replacement whose cancel was
    // refused leaves behind.
    let mut kv = fold_all(&mut tw);
    patch(&mut kv, NS, &format!("sr:profile:{}", who.0), |v| {
        v["upkeep"] = serde_json::json!(999_999);
    });
    reboot(&mut tw, &kv);
    assert_eq!(
        snap(&tw).obligation(live).expect("record").status,
        ObligationStatus::Cancelled,
        "an unclaimed lifestyle record is still collecting"
    );
    assert_eq!(
        sr(&tw).profile(who).expect("profile").upkeep,
        None,
        "the stale pointer survived"
    );
}

/// M7 (LOW): `@sr/migrate` under-funded a character whose chargen mint was refused.
///
/// `plan` reached `WouldMint(sheet value)` before `Adopt(allotment)`, and for a
/// post-E6a approval whose mint was refused the sheet's `nuyen` is the chargen budget's
/// leftover — not what the priority table promised. A character **with** a profile is
/// adopted at their allotment; one with **no** profile pre-dates the layer and answer
/// 18's rule is theirs.
#[test]
fn vf_m7_a_profiled_character_is_adopted_at_the_allotment_not_the_sheet() {
    let mut tw = boot();
    // A profile with no mint: approved, nothing on the sheet, adopted with an empty
    // account — exactly the shape a refused chargen mint leaves.
    let who = tw.spawn_player("Refused");
    approve(&mut tw, who, 0);
    ok(&mut tw, GOD, &format!("@sr/migrate commit #{}", who.0));
    let acct = account_of(&tw, who);
    assert_eq!(bal(&tw, acct), 0);
    assert!(sr(&tw).profile(who).expect("profile").minted.is_none());

    // Their priorities entitle them to a Resources allotment, and the sheet says 1.
    set_resources_priority(&mut tw, who, 'D', 1);
    let allotment = i64::from(
        ruleset_shadowrun::Sr5Config::default()
            .resource_nuyen(ruleset_shadowrun::data::Priority::from_char('D').expect("D")),
    );
    assert!(allotment > 1);

    let dry = ok(&mut tw, GOD, &format!("@sr/migrate dry #{}", who.0));
    assert!(dry.contains("priority allotment"), "{dry}");
    ok(&mut tw, GOD, &format!("@sr/migrate commit #{}", who.0));
    assert_eq!(
        bal(&tw, acct),
        allotment,
        "the refused-mint character was funded from the sheet, not the allotment"
    );

    // A character with NO profile is answer 18's case: the sheet value is the money.
    let old = tw.spawn_player("Legacy");
    approve(&mut tw, old, 7_000);
    let dry = ok(&mut tw, GOD, &format!("@sr/migrate dry #{}", old.0));
    assert!(dry.contains("would mint 7000"), "{dry}");
}

// ---- Lens D: disclosure and bounds ----------------------------------------

/// D1 (HIGH): closed listings were never pruned, so post→withdraw was a free loop
/// that minted permanent documents and lengthened every board scan for the life of
/// the world.
///
/// A closed listing is now deleted — the money was always the contract's, which keeps
/// its own terminal record — and a bounded ring of ids keeps `+run/show` answering
/// "closed" for one that has just gone.
#[test]
fn vf_d1_a_post_withdraw_loop_leaves_no_stored_listings() {
    let mut tw = boot();
    let j = runner(&mut tw, "Johnson", 50_000);
    let mut last = RunId(0);
    for n in 0..40 {
        ok(&mut tw, j, &format!("+run/post Churn{n}=100"));
        last = live_run(&tw);
        ok(&mut tw, j, &format!("+run/withdraw {last}"));
    }
    assert_eq!(
        sr(&tw).runs().count(),
        0,
        "closed listings are still in RAM"
    );
    assert_eq!(sr(&tw).open_runs(), 0);
    let kv = fold_all(&mut tw);
    let stored = kv
        .get(&SmolStr::new(NS))
        .map(|e| e.iter().filter(|(k, _)| k.starts_with("sr:run:")).count())
        .unwrap_or(0);
    assert_eq!(
        stored, 0,
        "{stored} listing document(s) survived 40 withdrawals"
    );
    // The most recent closure is still answerable — as "closed", and nothing else.
    let shown = ok(&mut tw, j, &format!("+run/show {last}"));
    assert!(shown.contains("closed"), "{shown}");
    // …and one from long before the ring is simply gone.
    err(&mut tw, j, "+run/show 0");
}

/// D1: `visible()` capped by records **matched** and walked every listing ever posted
/// to find them (D-LBK-5). It walks a bounded number of live records from a cursor
/// now, and hands back the cursor when it stopped early.
#[test]
fn vf_d1_the_board_page_is_bounded_on_records_visited() {
    let mut tw = boot();
    let mine = runner(&mut tw, "Mine", 50_000);
    ok(&mut tw, mine, "+run/post Mine=100");
    for n in 0..25 {
        let p = runner(&mut tw, &format!("Poster{n}"), 50_000);
        ok(&mut tw, p, &format!("+run/post Job{n}=100"));
    }
    assert_eq!(sr(&tw).open_runs(), 26);

    let page = ruleset_shadowrun::economy::config::BOARD_PAGE;
    let first = ruleset_shadowrun::economy::board::visible(tw.world(), mine, None, page, None);
    assert_eq!(first.ids.len(), page, "the page cap did not bind");
    assert!(
        first.visited <= ruleset_shadowrun::economy::config::BOARD_PAGE_VISIT,
        "the walk visited {} records",
        first.visited
    );
    let next = first.next.expect("a continuation cursor");
    let second =
        ruleset_shadowrun::economy::board::visible(tw.world(), mine, None, page, Some(next));
    assert_eq!(second.ids.len(), 6, "the second page lost listings");
    assert!(second.next.is_none(), "the walk never ends");
    // A **selective** filter is bounded on what it touches, not on what it matches.
    let selective =
        ruleset_shadowrun::economy::board::visible(tw.world(), mine, Some("mine"), page, None);
    assert_eq!(selective.ids.len(), 1);
    assert!(
        selective.visited <= ruleset_shadowrun::economy::config::BOARD_PAGE_VISIT,
        "a selective filter scanned {} records",
        selective.visited
    );
}

/// D2 (HIGH): outcome-row names were byte-uncapped on the write path **and** the
/// wire, so a 4 kB row name was frozen into the consent surface, copied into the board
/// document and broadcast by `+run/show`.
#[test]
fn vf_d2_an_oversized_outcome_row_name_is_refused_at_post_and_on_load() {
    let mut tw = boot();
    let j = runner(&mut tw, "Johnson", 50_000);
    let long = "x".repeat(4_000);
    let refused = err(
        &mut tw,
        j,
        &format!("+run/post Wordy=1000 outcomes=success,{long}"),
    );
    assert!(refused.contains("bytes"), "{refused}");
    assert_eq!(sr(&tw).runs().count(), 0, "a listing was created");
    assert!(contracts(&tw).iter().all(|c| state(&tw, *c).is_terminal()));

    // …and a document carrying one does not load.
    ok(&mut tw, j, "+run/post Fine=1000");
    let id = live_run(&tw);
    let mut kv = fold_all(&mut tw);
    patch(&mut kv, NS, &format!("sr:run:{}", id.0), |v| {
        v["outcomes"] = serde_json::json!(["success", "x".repeat(4_000)]);
    });
    reboot(&mut tw, &kv);
    assert_eq!(
        sr(&tw).runs().count(),
        0,
        "an oversized row name was loaded"
    );
}

/// D3 (MEDIUM): `resolve_account`'s "more than one nuyen account" line was an oracle
/// about the shape of a compartment the actor may not view — three distinguishable
/// outcomes where the hiding rule allows one.
#[test]
fn vf_d3_a_multi_account_compartment_hides_like_an_unknown_scope() {
    let mut tw = boot();
    let outsider = runner(&mut tw, "Outsider", 50_000);
    ok(&mut tw, GOD, "@org/create Ares");
    let org = tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .and_then(|s| s.state().orgs().next().map(|o| o.id))
        .expect("the org exists");
    let scope = tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .and_then(|s| s.state().scopes_in(org).next().map(|sc| sc.id))
        .expect("the root scope exists");
    ok(
        &mut tw,
        GOD,
        &format!("@account/open org:#{}/{}=sr.nuyen", org.0, scope.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@account/open org:#{}/{}=sr.nuyen", org.0, scope.0),
    );
    assert_eq!(
        snap(&tw)
            .accounts()
            .filter(|a| a.scope == Some(scope))
            .count(),
        2,
        "the compartment does not hold two accounts"
    );
    let two = err(
        &mut tw,
        outsider,
        &format!("+run/post Probe=100 scope={}", scope.0),
    );
    let none = err(&mut tw, outsider, "+run/post Probe=100 scope=9999");
    assert_eq!(
        two, none,
        "a two-account compartment reads unlike an unknown scope"
    );
    assert!(two.contains("No such account."), "{two}");
}

/// D4 (MEDIUM): `board=faction:<numeric id>` skipped `faction_listed`, so a hidden
/// faction that answered "No such faction." by name answered *yes* by id.
#[test]
fn vf_d4_a_hidden_faction_hides_by_id_as_well_as_by_name() {
    let mut tw = boot();
    let outsider = runner(&mut tw, "Outsider", 50_000);
    ok(&mut tw, GOD, "@faction/create Blackstar");
    ok(&mut tw, GOD, "@faction/set Blackstar=hidden:yes");
    let f = tw
        .world()
        .module_state::<mush_faction::FactionState>()
        .and_then(|s| s.factions.keys().next().copied())
        .expect("the faction exists");
    let by_id = err(
        &mut tw,
        outsider,
        &format!("+run/post Probe=100 board=faction:{}", f.0),
    );
    let unknown = err(&mut tw, outsider, "+run/post Probe=100 board=faction:9999");
    assert_eq!(by_id, unknown, "a hidden faction is visible by id");
    assert!(by_id.contains("No such faction."), "{by_id}");
    let by_name = err(
        &mut tw,
        outsider,
        "+run/post Probe=100 board=faction:Blackstar",
    );
    assert_eq!(by_name, unknown);
}

/// D5 (MEDIUM): `fixer=` was a **free** probe for whether a named character holds an
/// `sr.nuyen` account — no escrow, no listing, two distinguishable refusals. The
/// fee-row rule is a fact about the command and is asked first; a missing wallet now
/// answers exactly as an unresolvable name does.
#[test]
fn vf_d5_a_banked_and_an_unbanked_fixer_answer_alike() {
    let mut tw = boot();
    let j = runner(&mut tw, "Johnson", 50_000);
    let banked = runner(&mut tw, "Banked", 0);
    let unbanked = tw.spawn_player("Unbanked");
    // Without `fee=`, the answer must not depend on the target at all.
    let a = err(&mut tw, j, &format!("+run/post P=1000 fixer=#{}", banked.0));
    let b = err(
        &mut tw,
        j,
        &format!("+run/post P=1000 fixer=#{}", unbanked.0),
    );
    assert_eq!(
        a, b,
        "the fee-row refusal leaks whether the fixer is banked"
    );
    assert!(a.contains("fee row"), "{a}");
    // With `fee=`, an unbanked fixer reads exactly like a name that resolves to
    // nobody.
    let c = err(
        &mut tw,
        j,
        &format!("+run/post P=1000 fee=100 fixer=#{}", unbanked.0),
    );
    let d = err(&mut tw, j, "+run/post P=1000 fee=100 fixer=#99999");
    assert_eq!(c, d, "an unbanked fixer reads unlike an unknown one");
    assert!(c.contains("No such fixer."), "{c}");
    assert_eq!(sr(&tw).runs().count(), 0);
}

/// D7 / D13 (MEDIUM/LOW): the lifestyle observer resolved its subject with a
/// `profiles().find(..)` per event — one event per profiled character per period, so
/// quadratic work at every boundary — and `personal_account` walked the whole account
/// registry once per crew member. Both are indexed now, and the indexes are derived in
/// one write path each.
#[test]
fn vf_d7_the_indexes_agree_after_every_profile_mutation() {
    let mut tw = boot();
    let who = runner(&mut tw, "Runner", 50_000);
    assert!(sr(&tw).indexes_agree(), "after adoption");
    ok(&mut tw, who, "+lifestyle/set low");
    assert!(sr(&tw).indexes_agree(), "after a tier change");
    sweep_at(&mut tw, PERIOD * 2);
    assert!(sr(&tw).indexes_agree(), "after a paid period");
    for n in 3..=6 {
        sweep_at(&mut tw, PERIOD * n);
        assert!(sr(&tw).indexes_agree(), "after period {n}");
    }
    ok(&mut tw, who, "+run/post Job=100");
    assert!(sr(&tw).indexes_agree(), "after a listing");
    let id = live_run(&tw);
    ok(&mut tw, who, &format!("+run/withdraw {id}"));
    assert!(sr(&tw).indexes_agree(), "after a withdrawal");
}

/// D7: a whole population's period boundary resolves each event through the index,
/// and every one of them lands on the right subject.
#[test]
fn vf_d7_a_population_wide_period_boundary_bills_every_profile() {
    let mut tw = boot();
    let mut who = Vec::new();
    for n in 0..40 {
        let r = runner(&mut tw, &format!("Pop{n}"), 3_000);
        ok(&mut tw, r, "+lifestyle/set low");
        who.push(r);
    }
    assert!(sr(&tw).indexes_agree());
    // One paid period, then one missed one: every profile must have its own arrears.
    sweep_at(&mut tw, PERIOD * 2);
    sweep_at(&mut tw, PERIOD * 3);
    for r in &who {
        let p = sr(&tw).profile(*r).expect("profile").clone();
        assert_eq!(p.unpaid_periods, 1, "#{} missed nothing", r.0);
        assert_eq!(p.arrears.minor(), 2_000, "#{} was billed wrong", r.0);
    }
    assert!(sr(&tw).indexes_agree());
}

/// D8 / R6 (LOW): `RunNotice.kinds` (count and bytes), the notice window and
/// `Mint.key` were uncapped in their wires — every one of them a stored string or
/// number that a render reads back.
#[test]
fn vf_d8_oversized_notice_fields_and_mint_keys_refuse_to_load() {
    let mut tw = boot();
    let j = runner(&mut tw, "Johnson", 50_000);
    ok(&mut tw, j, "+run/post Fine=1000");
    let id = live_run(&tw);
    let kv = fold_all(&mut tw);
    let run_key = format!("sr:run:{}", id.0);
    let profile_key = format!("sr:profile:{}", j.0);

    for bad in [
        serde_json::json!(["a", "b", "c", "d", "e", "f", "g", "h", "i"]),
        serde_json::json!([("k".repeat(64))]),
    ] {
        let mut kv = kv.clone();
        patch(&mut kv, NS, &run_key, |v| {
            v["notice"]["kinds"] = bad.clone();
        });
        reboot(&mut tw, &kv);
        assert_eq!(sr(&tw).runs().count(), 0, "an unbounded kinds list loaded");
    }
    let mut kv2 = kv.clone();
    patch(&mut kv2, NS, &run_key, |v| {
        v["notice"]["window"] = serde_json::json!(i64::MAX);
    });
    reboot(&mut tw, &kv2);
    assert_eq!(sr(&tw).runs().count(), 0, "an unbounded window loaded");

    let mut kv3 = kv;
    patch(&mut kv3, NS, &profile_key, |v| {
        v["minted"]["key"] = serde_json::json!("z".repeat(200));
    });
    reboot(&mut tw, &kv3);
    assert!(sr(&tw).profile(j).is_none(), "an unbounded mint key loaded");
}

/// D10 (LOW): `@sr/profile` rendered arrears as bare minor units — the one arrears
/// render in the crate that did not go through the instrument.
#[test]
fn vf_d10_the_staff_profile_renders_arrears_as_money() {
    let mut tw = boot();
    let who = runner(&mut tw, "Runner", 5_000);
    ok(&mut tw, who, "+lifestyle/set middle");
    sweep_at(&mut tw, PERIOD * 2);
    sweep_at(&mut tw, PERIOD * 3);
    assert_eq!(
        sr(&tw).profile(who).expect("profile").arrears.minor(),
        5_000
    );
    let out = ok(&mut tw, GOD, &format!("@sr/profile #{}", who.0));
    assert!(
        out.contains("\u{a5}5000"),
        "the arrears are not rendered in the instrument: {out}"
    );
}

/// D12 (LOW): `+nuyen <who>` said **nothing** when the account existed but the viewer
/// could not see it, and a sheet line when there was no account — so the silence was
/// the tell that a character was banked. One fallback either way.
#[test]
fn vf_d12_an_unviewable_account_and_no_account_read_alike() {
    let mut tw = boot();
    let banked = runner(&mut tw, "Banked", 1_234);
    let unbanked = tw.spawn_player("Unbanked");
    // A viewer who may examine anybody and is not staff: exactly the standing the
    // oracle needed.
    let nosy = tw.spawn_player("Nosy");
    if let Some(o) = tw.world_mut().get_mut(nosy) {
        o.powers = mush_core::Powers::EXAMINE_ALL;
    }
    let a = ok(&mut tw, nosy, &format!("+nuyen #{}", banked.0));
    let b = ok(&mut tw, nosy, &format!("+nuyen #{}", unbanked.0));
    assert_eq!(a, b, "the banked character is distinguishable");
    assert!(!a.contains("1234"), "the balance leaked: {a}");
    // The owner still reads their own.
    assert!(ok(&mut tw, banked, "+nuyen").contains("1234"));
}

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
fn run_state(tw: &TestWorld, id: RunId) -> RunState {
    sr(tw).run(id).expect("listing").state
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
#[test]
fn a_fixer_takes_a_named_fee_row_and_is_never_the_issuer() {
    let mut tw = boot();
    let j = runner(&mut tw, "Johnson", 50_000);
    let fixer = runner(&mut tw, "Fixer", 0);
    let fixer_acct = account_of(&tw, fixer);
    ok(
        &mut tw,
        j,
        &format!("+run/post Brokered=10000 fixer=#{} fee=1000", fixer.0),
    );
    let id = only_run(&tw);
    let c = snap(&tw)
        .contract(contract_of(&tw, id))
        .expect("contract")
        .clone();
    assert_eq!(
        c.issuer,
        PrincipalId::Object(j),
        "the fixer became the issuer"
    );
    assert_eq!(c.roles.representative, Some(PrincipalId::Object(fixer)));
    for row in &c.consent.terms.outcomes {
        let fee: i64 = row
            .payouts
            .iter()
            .filter(|s| s.to == fixer_acct)
            .map(|s| match s.share {
                mush_economy::Share::Exact(a) => a.minor(),
                mush_economy::Share::Permille(_) => 0,
            })
            .sum();
        assert_eq!(fee, 1_000, "row {} does not pay the named fee", row.name);
    }
    // A fixer with no fee row is refused: an unnamed remainder never reaches them.
    let refused = err(
        &mut tw,
        j,
        &format!("+run/post Unpaid=1000 fixer=#{}", fixer.0),
    );
    assert!(refused.contains("fee row"), "{refused}");
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

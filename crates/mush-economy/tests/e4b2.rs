//! E4b2 acceptance (08c §5, scenarios 16–17): **contracts live**.
//!
//! The verify axis of this cut is *consent forgery, settle-twice, hidden parties*,
//! so the rhythms here are:
//!
//! - **two rights, checked separately.** Accepting a contract and being able to fund
//!   it are different questions, and every test that touches an org asks both.
//! - **the escrow is the contract's.** No command may release or capture it; the
//!   only ways out are the contract's own settlement, refund and forfeit.
//! - **nothing moved.** Every refusal is asserted against balances, `held_amount`,
//!   object locations and the journal, not just against the message.
//! - **fold and reboot.** Every state survives a restart, a settled contract whose
//!   transaction the journal does not have freezes the economy, and an `Active`
//!   contract whose escrow is gone comes back `Disputed`.
//!
//! Everything runs through the real `Engine` via `TestWorld`, like `e4b1.rs`.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use mush_core::{Audience, GOD, LIMBO, ModuleBootData, ModuleWrite, ObjId, ObjKind};
use mush_economy::{
    AccountId, AccountStatus, Amount, ContractAuth, ContractId, ContractState, EconomySnapshot,
    EconomyStore, Encumbrance, EscalationSubject, HoldId, HoldPurpose, ObligationCause,
    ObligationStatus, VaultId,
};
use mush_org::{CapabilityId, OrgId, OrgScopeId, OrgStore, PrincipalId};
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, text_of};
use smol_str::SmolStr;

const NS: &str = "economy";

// ---- rig ------------------------------------------------------------------

/// The capability vocabulary the server freezes at wiring, reproduced here so the
/// `contract.*` entries are exercised through the real configuration.
fn vocabulary() -> BTreeSet<CapabilityId> {
    mush_economy::CAPABILITIES
        .iter()
        .copied()
        .chain(["org.role.manage", "org.grant.delegate"])
        .map(|c| CapabilityId::new(c).expect("well-formed"))
        .collect()
}

fn boot() -> TestWorld {
    TestWorld::with_modules(vec![
        Box::new(
            mush_org::OrgModule::new()
                .with_capabilities(vocabulary())
                .with_scope_use(vec![Arc::new(mush_economy::EconomyScopeUse)]),
        ),
        Box::new(mush_economy::EconomyModule::with_custody(vec![])),
    ])
}

fn texts(events: &[(Audience, OutputEvent)]) -> String {
    events.iter().map(|(_, e)| text_of(e)).collect::<Vec<_>>().join("\n")
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
    assert!(has_error(&ev), "`{line}` should have been refused, got: {}", texts(&ev));
    texts(&ev)
}
fn store(tw: &TestWorld) -> &EconomyStore {
    tw.world().module_state::<EconomyStore>().expect("economy store")
}
fn snap(tw: &TestWorld) -> &EconomySnapshot {
    store(tw).snapshot()
}
fn bal(tw: &TestWorld, id: AccountId) -> i64 {
    snap(tw).account(id).expect("account").posted_balance.minor()
}
fn held(tw: &TestWorld, id: AccountId) -> i64 {
    snap(tw).account(id).expect("account").held_amount.minor()
}
fn journal_len(tw: &TestWorld) -> usize {
    store(tw).transactions().count()
}
fn contract(tw: &TestWorld, id: ContractId) -> mush_economy::EconomicContract {
    snap(tw).contract(id).expect("contract").clone()
}
fn state(tw: &TestWorld, id: ContractId) -> ContractState {
    contract(tw, id).state
}
fn thing(tw: &mut TestWorld, name: &str) -> ObjId {
    tw.world_mut().create(ObjKind::Thing, name, GOD, Some(LIMBO)).expect("create")
}
fn loc(tw: &TestWorld, o: ObjId) -> Option<ObjId> {
    tw.world().get(o).and_then(|x| x.location)
}
fn account_of(tw: &TestWorld, owner: ObjId) -> AccountId {
    snap(tw)
        .accounts()
        .find(|a| a.owner == PrincipalId::Object(owner))
        .expect("account of owner")
        .id
}
fn only_contract(tw: &TestWorld) -> ContractId {
    let ids: Vec<ContractId> = snap(tw).contracts().map(|c| c.id).collect();
    assert_eq!(ids.len(), 1, "expected exactly one contract, got {ids:?}");
    ids[0]
}

/// One sweep fire at wall-clock second `t`.
fn sweep_at(tw: &mut TestWorld, t: i64) -> Vec<(Audience, OutputEvent)> {
    tw.set_now(t);
    tw.fire_due(t.saturating_mul(1000))
}

fn fold_kv(tw: &mut TestWorld) -> Vec<(SmolStr, serde_json::Value)> {
    let mut kv: Vec<(SmolStr, serde_json::Value)> = Vec::new();
    merge(&mut kv, drain(tw));
    kv
}
fn drain(tw: &mut TestWorld) -> Vec<(SmolStr, serde_json::Value)> {
    let mut kv: Vec<(SmolStr, serde_json::Value)> = Vec::new();
    for op in tw.take_module_ops() {
        if op.ns != NS {
            continue;
        }
        match op.op {
            ModuleWrite::JsonSet { key, value } => {
                kv.retain(|(k, _)| *k != key);
                kv.push((key, value));
            }
            ModuleWrite::Del { key } => kv.retain(|(k, _)| *k != key),
            ModuleWrite::StreamAppend { .. } => {}
        }
    }
    kv
}
fn merge(base: &mut Vec<(SmolStr, serde_json::Value)>, extra: Vec<(SmolStr, serde_json::Value)>) {
    for (k, v) in extra {
        base.retain(|(bk, _)| *bk != k);
        base.push((k, v));
    }
}
fn reboot(tw: &mut TestWorld, kv: Vec<(SmolStr, serde_json::Value)>) {
    tw.modules_loaded(HashMap::from([(
        SmolStr::new(NS),
        ModuleBootData { kv, streams: HashMap::new() },
    )]));
}
fn patch(kv: &mut [(SmolStr, serde_json::Value)], key: &str, f: impl Fn(&mut serde_json::Value)) {
    let mut seen = false;
    for (k, v) in kv.iter_mut() {
        if k == key {
            f(v);
            seen = true;
        }
    }
    assert!(seen, "no such persisted key to patch: {key}");
}

/// A Johnson, three runners, a watcher, an instrument and funded wallets.
struct Run {
    johnson: ObjId,
    a: ObjId,
    b: ObjId,
    c: ObjId,
    watcher: ObjId,
    j_acct: AccountId,
    a_acct: AccountId,
    b_acct: AccountId,
    c_acct: AccountId,
}

fn run_rig(tw: &mut TestWorld, funds: i64) -> Run {
    let johnson = tw.spawn_player("Johnson");
    let a = tw.spawn_player("Alder");
    let b = tw.spawn_player("Bex");
    let c = tw.spawn_player("Cyl");
    let watcher = tw.spawn_player("Watcher");
    ok(tw, GOD, "@instrument/register nuyen=0");
    ok(tw, GOD, "@account/system faucet=nuyen");
    for who in [johnson, a, b, c, watcher] {
        ok(tw, GOD, &format!("@account/open #{}=nuyen", who.0));
    }
    let faucet = snap(tw)
        .accounts()
        .find(|x| matches!(x.owner, PrincipalId::System(_)))
        .expect("faucet")
        .id;
    let j_acct = account_of(tw, johnson);
    if funds > 0 {
        ok(tw, GOD, &format!("@econ/mint {faucet}={j_acct} {funds}"));
    }
    Run {
        johnson,
        a,
        b,
        c,
        watcher,
        j_acct,
        a_acct: account_of(tw, a),
        b_acct: account_of(tw, b),
        c_acct: account_of(tw, c),
    }
}

/// The whole scenario-16 run, built and offered but not yet accepted.
fn offered_run(tw: &mut TestWorld, r: &Run, pay: i64) -> ContractId {
    ok(tw, r.johnson, "@contract/draft Extraction");
    let id = only_contract(tw);
    for who in [r.a, r.b, r.c] {
        ok(tw, r.johnson, &format!("@contract/party {id}=#{}", who.0));
    }
    ok(tw, r.johnson, &format!("@contract/pay {id}={} {pay}", r.j_acct));
    ok(
        tw,
        r.johnson,
        &format!("@contract/split {id}={} 333‰ {} 333‰ {} 333‰", r.a_acct, r.b_acct, r.c_acct),
    );
    ok(tw, r.johnson, &format!("@contract/remainder {id}={}", r.j_acct));
    ok(tw, r.johnson, &format!("@contract/condition {id}=attest party:#{}", r.johnson.0));
    ok(tw, r.johnson, &format!("@contract/offer {id}"));
    id
}

fn accept_all(tw: &mut TestWorld, r: &Run, id: ContractId) {
    for who in [r.johnson, r.a, r.b, r.c] {
        ok(tw, who, &format!("@contract/accept {id}"));
    }
}

// ======================================================================
// Scenario 16 — the run, end to end
// ======================================================================

#[test]
fn a_run_drafts_escrows_attests_and_settles_its_split_exactly_once() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    let id = offered_run(&mut tw, &r, 300);
    assert_eq!(state(&tw, id), ContractState::Offered);

    // The Johnson's escrow is placed by HIS accept, not at draft time.
    assert_eq!(held(&tw, r.j_acct), 0, "escrow was reserved before anybody accepted");
    ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
    assert_eq!(held(&tw, r.j_acct), 300, "the accept did not place the consideration");
    assert_eq!(bal(&tw, r.j_acct), 3_000, "a reservation moved money");
    assert_eq!(state(&tw, id), ContractState::Offered, "one acceptance activated it");

    for who in [r.a, r.b, r.c] {
        ok(&mut tw, who, &format!("@contract/accept {id}"));
    }
    assert_eq!(state(&tw, id), ContractState::Active, "the last acceptance did not activate it");

    let before = journal_len(&tw);
    ok(&mut tw, r.johnson, &format!("@contract/attest {id}=0"));
    assert!(
        matches!(state(&tw, id), ContractState::Settled { .. }),
        "the step that satisfied the last condition did not settle: {:?}",
        state(&tw, id)
    );
    // 333‰ of 300 floors to 99 each; the remaining 3 goes where the parties said.
    assert_eq!(bal(&tw, r.a_acct), 99);
    assert_eq!(bal(&tw, r.b_acct), 99);
    assert_eq!(bal(&tw, r.c_acct), 99);
    assert_eq!(bal(&tw, r.j_acct), 3_000 - 300 + 3, "the remainder did not land where declared");
    assert_eq!(held(&tw, r.j_acct), 0, "the escrow is still reserved after settlement");
    assert_eq!(journal_len(&tw), before + 1, "settlement was not one transaction");

    // A second attest, and therefore a second settlement, moves nothing.
    let after = journal_len(&tw);
    err(&mut tw, r.johnson, &format!("@contract/attest {id}=0"));
    assert_eq!(journal_len(&tw), after, "a second attest posted something");
    assert_eq!(bal(&tw, r.a_acct), 99, "a second settlement paid twice");
}

#[test]
fn the_settlement_key_is_deterministic_and_the_plan_guards_every_escrow_revision() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    // A contract with no conditions at all: `Active` the moment everyone accepts, and
    // settled by the next sweep — which is what lets the plan be inspected first.
    ok(&mut tw, r.johnson, "@contract/draft Plain");
    let id = only_contract(&tw);
    ok(&mut tw, r.johnson, &format!("@contract/party {id}=#{}", r.a.0));
    ok(&mut tw, r.johnson, &format!("@contract/pay {id}={} 300", r.j_acct));
    ok(&mut tw, r.johnson, &format!("@contract/split {id}={} 1000‰", r.a_acct));
    ok(&mut tw, r.johnson, &format!("@contract/remainder {id}={}", r.j_acct));
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
    ok(&mut tw, r.a, &format!("@contract/accept {id}"));

    // The guard the exit criterion's "stale revisions" rests on: the settlement plan
    // pins each escrow hold's revision, so a reservation that moved between building
    // the plan and posting it is `StaleHold`, never applied to a different hold.
    let c = contract(&tw, id);
    let holds: Vec<HoldId> = c.escrow.clone();
    assert_eq!(holds.len(), 1);
    let plan = {
        let s = snap(&tw);
        let env = mush_economy::ValidateEnv {
            auth: &mush_economy::AllowAll,
            custody: &mush_economy::NoCustody,
            facts: &mush_economy::NoFacts,
            now: 0,
            ic_now: 0,
            next_hold_id: s.next_hold_floor(),
        };
        mush_economy::settlement_plan(&c, s, &env).expect("a condition-less contract settles")
    };
    let live = snap(&tw).hold(holds[0]).expect("escrow").revision;
    assert_eq!(
        plan.expected_hold_revisions,
        vec![mush_economy::HoldRevision { hold: holds[0], revision: live }],
        "the settlement plan carries no staleness guard over its escrow"
    );
    assert_eq!(plan.idempotency, mush_economy::settle_key(id));

    sweep_at(&mut tw, 100);
    let ContractState::Settled { transaction } = state(&tw, id) else {
        panic!("the sweep did not settle a ready contract")
    };
    let record =
        snap(&tw).idempotency(&mush_economy::settle_key(id)).expect("the deterministic settle key");
    assert_eq!(record.transaction, transaction, "the settlement did not use contract/{{id}}/settle");
    let tx = store(&tw).transaction(transaction).expect("the settlement transaction");
    assert_eq!(tx.hold_effects.len(), 1, "the settlement captured the wrong number of holds");
    assert_eq!(bal(&tw, r.a_acct), 300);

    // A second fire at the same instant does nothing at all: the contract is settled,
    // and its deterministic key would resolve to the same transaction anyway.
    let before = journal_len(&tw);
    sweep_at(&mut tw, 100);
    assert_eq!(journal_len(&tw), before, "a second sweep settled twice");
    assert_eq!(bal(&tw, r.a_acct), 300);
}

#[test]
fn a_party_who_did_not_accept_is_never_debited() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    // Two payers; only one of them accepts.
    ok(&mut tw, r.johnson, "@contract/draft Joint");
    let id = only_contract(&tw);
    ok(&mut tw, GOD, &format!("@econ/mint 0={} 500", r.a_acct));
    ok(&mut tw, r.johnson, &format!("@contract/party {id}=#{}", r.a.0));
    ok(&mut tw, r.johnson, &format!("@contract/pay {id}={} 300", r.j_acct));
    ok(&mut tw, r.johnson, &format!("@contract/pay {id}={} 200", r.a_acct));
    ok(&mut tw, r.johnson, &format!("@contract/split {id}={} 1000‰", r.b_acct));
    ok(&mut tw, r.johnson, &format!("@contract/remainder {id}={}", r.j_acct));
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));

    assert_eq!(state(&tw, id), ContractState::Offered);
    assert_eq!(held(&tw, r.a_acct), 0, "a party who never accepted had funds reserved");
    assert_eq!(bal(&tw, r.a_acct), 500, "a party who never accepted was debited");
    assert_eq!(bal(&tw, r.b_acct), 0, "an unaccepted contract paid out");
}

#[test]
fn a_consent_surface_mutation_after_offer_is_refused() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    let id = offered_run(&mut tw, &r, 300);
    let hash = contract(&tw, id).consent_hash;

    for line in [
        format!("@contract/party {id}=#{}", r.watcher.0),
        format!("@contract/pay {id}={} 10", r.j_acct),
        format!("@contract/split {id}={} 500‰", r.a_acct),
        format!("@contract/visibility {id}=public"),
        format!("@contract/deadline {id}=600"),
        format!("@contract/condition {id}=attest staff"),
    ] {
        err(&mut tw, r.johnson, &line);
    }
    assert_eq!(contract(&tw, id).consent_hash, hash, "the frozen consent hash moved");
    assert!(contract(&tw, id).consent_intact());
}

// ======================================================================
// The two rights: accepting, and being able to fund it
// ======================================================================

struct Org {
    org: OrgId,
    scope: OrgScopeId,
    acct: AccountId,
    member: ObjId,
    payee: ObjId,
    payee_acct: AccountId,
}

fn org_rig(tw: &mut TestWorld) -> Org {
    let member = tw.spawn_player("Quinn");
    let payee = tw.spawn_player("Pat");
    ok(tw, GOD, "@instrument/register gold=0");
    ok(tw, GOD, "@org/create Ironhold");
    let org = tw
        .world()
        .module_state::<OrgStore>()
        .expect("org store")
        .state()
        .orgs()
        .next()
        .expect("the org")
        .id;
    ok(tw, GOD, "@org/scope create Ironhold=operations");
    let scope = tw
        .world()
        .module_state::<OrgStore>()
        .expect("org store")
        .state()
        .scopes_in(org)
        .find(|s| s.name == "operations")
        .expect("scope")
        .id;
    ok(tw, GOD, "@account/system faucet=gold");
    ok(tw, GOD, &format!("@account/open org:Ironhold/{}=gold", scope.0));
    ok(tw, GOD, &format!("@account/open #{}=gold", payee.0));
    let acct = snap(tw).accounts().find(|a| a.scope == Some(scope)).expect("org account").id;
    let payee_acct = account_of(tw, payee);
    let faucet = snap(tw)
        .accounts()
        .find(|x| matches!(x.owner, PrincipalId::System(_)))
        .expect("faucet")
        .id;
    ok(tw, GOD, &format!("@econ/mint {faucet}={acct} 1000"));
    Org { org, scope, acct, member, payee, payee_acct }
}

#[test]
fn contract_accept_needs_the_capability_and_debit_authority_independently() {
    let mut tw = boot();
    let o = org_rig(&mut tw);
    // Staff drafts an ORG contract on the compartment, so the org is the party.
    ok(&mut tw, GOD, &format!("@contract/draft Supply scope=#{}", o.scope.0));
    let id = only_contract(&tw);
    ok(&mut tw, GOD, &format!("@contract/party {id}=#{}", o.payee.0));
    ok(&mut tw, GOD, &format!("@contract/pay {id}={} 400", o.acct));
    ok(&mut tw, GOD, &format!("@contract/split {id}={} 1000‰", o.payee_acct));
    ok(&mut tw, GOD, &format!("@contract/remainder {id}={}", o.acct));
    ok(&mut tw, GOD, &format!("@contract/condition {id}=attest staff"));
    ok(&mut tw, GOD, &format!("@contract/offer {id}"));

    // No capability at all: the contract does not exist as far as Quinn is concerned.
    let seen = err(&mut tw, o.member, &format!("@contract/accept {id}"));
    assert!(seen.contains("No such contract."), "wrong refusal: {seen}");

    // `contract.accept` alone lets her SEE it and try — and she still cannot escrow
    // the org's money, because that is a separate right (D-E4-4).
    ok(&mut tw, GOD, &format!("@org/grant #{}=Ironhold contract.accept scope=#{}", o.member.0, o.scope.0));
    err(&mut tw, o.member, &format!("@contract/accept {id}"));
    assert_eq!(held(&tw, o.acct), 0, "an accept with no debit authority escrowed org funds");
    assert!(contract(&tw, id).acceptances.is_empty(), "a refused accept recorded an acceptance");

    // Both rights: now it works.
    ok(&mut tw, GOD, &format!("@org/grant #{}=Ironhold economy.account.post scope=#{}", o.member.0, o.scope.0));
    ok(&mut tw, o.member, &format!("@contract/accept {id}"));
    assert_eq!(held(&tw, o.acct), 400, "the accept did not place the org's consideration");
    assert_eq!(contract(&tw, id).acceptances.len(), 1);
    let _ = o.org;
}

#[test]
fn an_approval_rule_on_the_payers_account_records_no_acceptance() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    // D-E4-11: a placement is a debit-authority plan, so the account's rule applies —
    // and an accept that needs approval leaves the contract exactly where it was.
    ok(&mut tw, GOD, &format!("@account/approval {}=10 named:1:#{}", r.j_acct, r.watcher.0));
    let id = offered_run(&mut tw, &r, 300);
    let seen = ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
    assert!(seen.contains("Approval required"), "wrong outcome: {seen}");
    assert!(contract(&tw, id).acceptances.is_empty(), "an unapproved accept was recorded");
    assert_eq!(held(&tw, r.j_acct), 0, "an unapproved accept reserved funds");
    assert_eq!(state(&tw, id), ContractState::Offered);
}

// ======================================================================
// ContractAuth: the exact set of operations
// ======================================================================

#[test]
fn contract_auth_grants_no_debit_no_stranger_credit_and_no_escrow_release() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    let id = offered_run(&mut tw, &r, 300);
    accept_all(&mut tw, &r, id);

    use mush_economy::AuthEval;
    let c = contract(&tw, id);
    let s = snap(&tw);
    let settling = ContractAuth::new(&c, s);
    let payer = s.account(r.j_acct).expect("account");
    let stranger = s.account(account_of(&tw, r.watcher)).expect("account");
    let payee = s.account(r.a_acct).expect("account");
    let actor = PrincipalId::Object(r.johnson);

    // No bare debit authority at all, on any account, ever.
    assert!(!settling.authorize(&actor, payer, Amount::from_minor(-1)));
    assert!(!settling.authorize(&actor, payee, Amount::from_minor(-1)));
    // A credit only to a declared payee — an extra line to a stranger is refused.
    assert!(settling.authorize(&actor, payee, Amount::from_minor(1)));
    assert!(!settling.authorize(&actor, stranger, Amount::from_minor(1)));
    // The escrow hold: a settlement captures and may not release.
    let hold = s.hold(c.escrow[0]).expect("escrow");
    assert!(settling.authorize_hold(&actor, hold, &mush_economy::HoldOp::Capture {
        hold: hold.id,
        amount: hold.amount,
    }));
    assert!(!settling.authorize_hold(&actor, hold, &mush_economy::HoldOp::Release { hold: hold.id }));
    // ...and an unwind is the other way round.
    let unwinding = ContractAuth::unwinding(&c, s);
    assert!(unwinding.authorize_hold(&actor, hold, &mush_economy::HoldOp::Release { hold: hold.id }));
    assert!(!unwinding.authorize_hold(&actor, hold, &mush_economy::HoldOp::Capture {
        hold: hold.id,
        amount: hold.amount,
    }));
    assert!(!unwinding.authorize(&actor, payee, Amount::from_minor(1)), "an unwind credited somebody");
    // A capture over the hold's amount is refused whatever the mode.
    let over = hold.amount.checked_add(Amount::from_minor(1)).expect("fits");
    assert!(!settling.authorize_hold(&actor, hold, &mush_economy::HoldOp::Capture {
        hold: hold.id,
        amount: over,
    }));
}

#[test]
fn a_contracts_escrow_cannot_be_released_or_captured_by_hand() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    let id = offered_run(&mut tw, &r, 300);
    accept_all(&mut tw, &r, id);
    let hold = contract(&tw, id).escrow[0];
    assert!(matches!(
        snap(&tw).hold(hold).expect("escrow").purpose,
        HoldPurpose::Escrow { .. }
    ));

    // The placer, the payee, and staff: all refused, all for the same reason.
    for who in [r.johnson, r.a, GOD] {
        let seen = err(&mut tw, who, &format!("@hold/release {hold}"));
        assert!(seen.contains("escrow") || seen.contains("No such hold."), "wrong refusal: {seen}");
        err(&mut tw, who, &format!("@hold/capture {hold}={} 100", r.a_acct));
    }
    assert_eq!(held(&tw, r.j_acct), 300, "a by-hand release freed a contract's escrow");
    assert_eq!(bal(&tw, r.a_acct), 0, "a by-hand capture spent a contract's escrow");
}

// ======================================================================
// A sale: title, both sides, and live re-verification
// ======================================================================

/// A buyer, a seller, and a titled widget the seller owns.
struct Sale {
    buyer: ObjId,
    seller: ObjId,
    buyer_acct: AccountId,
    seller_acct: AccountId,
    widget: ObjId,
    id: ContractId,
}

fn sale(tw: &mut TestWorld) -> Sale {
    let buyer = tw.spawn_player("Buyer");
    let seller = tw.spawn_player("Seller");
    ok(tw, GOD, "@instrument/register gold=0");
    ok(tw, GOD, "@account/system faucet=gold");
    ok(tw, GOD, &format!("@account/open #{}=gold", buyer.0));
    ok(tw, GOD, &format!("@account/open #{}=gold", seller.0));
    let faucet = snap(tw)
        .accounts()
        .find(|x| matches!(x.owner, PrincipalId::System(_)))
        .expect("faucet")
        .id;
    let buyer_acct = account_of(tw, buyer);
    let seller_acct = account_of(tw, seller);
    ok(tw, GOD, &format!("@econ/mint {faucet}={buyer_acct} 500"));
    let widget = thing(tw, "widget");
    ok(tw, GOD, &format!("@asset/title #{}=#{}", widget.0, seller.0));

    ok(tw, buyer, "@contract/draft Sale");
    let id = only_contract(tw);
    ok(tw, buyer, &format!("@contract/party {id}=#{}", seller.0));
    ok(tw, buyer, &format!("@contract/pay {id}={buyer_acct} 200"));
    ok(tw, buyer, &format!("@contract/split {id}={seller_acct} 1000‰"));
    ok(tw, buyer, &format!("@contract/remainder {id}={seller_acct}"));
    ok(
        tw,
        buyer,
        &format!("@contract/deliver {id}=title #{} #{}->#{}", widget.0, seller.0, buyer.0),
    );
    // The condition is the buyer confirming delivery; the seller's title is not a
    // condition but a **deliverable**, so the live check that matters at settlement
    // is D-E3-12's authority one: `ContractAuth` may only move a title whose current
    // owner is the party the surface named.
    ok(tw, buyer, &format!("@contract/condition {id}=attest party:#{}", buyer.0));
    ok(tw, buyer, &format!("@contract/offer {id}"));
    ok(tw, buyer, &format!("@contract/accept {id}"));
    ok(tw, seller, &format!("@contract/accept {id}"));
    Sale { buyer, seller, buyer_acct, seller_acct, widget, id }
}

#[test]
fn a_sale_moves_money_and_title_in_one_bundle_with_both_sides_accepted() {
    let mut tw = boot();
    let s = sale(&mut tw);
    assert_eq!(state(&tw, s.id), ContractState::Active);
    ok(&mut tw, s.buyer, &format!("@contract/attest {}=0", s.id));
    assert!(matches!(state(&tw, s.id), ContractState::Settled { .. }), "the sale did not settle");
    assert_eq!(bal(&tw, s.buyer_acct), 300);
    assert_eq!(bal(&tw, s.seller_acct), 200);
    assert_eq!(
        snap(&tw).asset(s.widget).expect("asset").title_owner,
        PrincipalId::Object(s.buyer),
        "title did not move with the money"
    );
    let _ = s.seller;
}

#[test]
fn a_seller_who_loses_title_before_settlement_settles_nothing_and_is_disputed() {
    let mut tw = boot();
    let s = sale(&mut tw);
    // The seller hands the title elsewhere between acceptance and settlement, then
    // the buyer confirms delivery. `ContractAuth` may only move a title FROM the
    // party the consent surface named, so the settlement is refused every time.
    let third = tw.spawn_player("Third");
    ok(&mut tw, GOD, &format!("@asset/transfer #{}=#{}", s.widget.0, third.0));
    ok(&mut tw, s.buyer, &format!("@contract/attest {}=0", s.id));
    assert_eq!(state(&tw, s.id), ContractState::ReadyToSettle, "a refused settlement moved on");

    // Every fire re-verifies, refuses, and moves nothing; the attempts are bounded.
    for k in 0..(u32::from(mush_economy::MAX_SETTLE_ATTEMPTS) + 1) {
        sweep_at(&mut tw, 100 + i64::from(k) * 60);
    }
    assert_eq!(bal(&tw, s.buyer_acct), 500, "a failed re-verification moved money");
    assert_eq!(bal(&tw, s.seller_acct), 0, "a failed re-verification paid the seller");
    assert_eq!(held(&tw, s.buyer_acct), 200, "the escrow was released by a failure");
    assert_eq!(state(&tw, s.id), ContractState::Disputed, "the attempts were not bounded");
    assert!(
        store(&tw)
            .escalations()
            .any(|e| e.subject == EscalationSubject::Contract(s.id)),
        "no level-2 record was opened for the exhausted settlement"
    );
}

// ======================================================================
// Custody: the lock, and the refund that returns the deposit
// ======================================================================

struct Deposit {
    seller: ObjId,
    buyer: ObjId,
    buyer_acct: AccountId,
    seller_acct: AccountId,
    widget: ObjId,
    vault: VaultId,
    id: ContractId,
}

fn deposit_rig(tw: &mut TestWorld) -> Deposit {
    let buyer = tw.spawn_player("Buyer");
    let seller = tw.spawn_player("Seller");
    ok(tw, GOD, "@instrument/register gold=0");
    ok(tw, GOD, "@account/system faucet=gold");
    ok(tw, GOD, &format!("@account/open #{}=gold", buyer.0));
    ok(tw, GOD, &format!("@account/open #{}=gold", seller.0));
    let faucet = snap(tw)
        .accounts()
        .find(|x| matches!(x.owner, PrincipalId::System(_)))
        .expect("faucet")
        .id;
    let buyer_acct = account_of(tw, buyer);
    let seller_acct = account_of(tw, seller);
    ok(tw, GOD, &format!("@econ/mint {faucet}={buyer_acct} 500"));

    let chest = thing(tw, "chest");
    ok(tw, GOD, &format!("@vault/create escrowbox=container:#{} owner=#{}", chest.0, seller.0));
    let vault = snap(tw).vaults().next().expect("vault").id;
    let widget = thing(tw, "widget");
    tw.world_mut().move_to(widget, seller).expect("hand it to the seller");
    ok(tw, seller, &format!("@vault/deposit #{}={vault}", widget.0));

    ok(tw, buyer, "@contract/draft Delivery");
    let id = only_contract(tw);
    ok(tw, buyer, &format!("@contract/party {id}=#{}", seller.0));
    ok(tw, buyer, &format!("@contract/pay {id}={buyer_acct} 200"));
    ok(tw, buyer, &format!("@contract/split {id}={seller_acct} 1000‰"));
    ok(tw, buyer, &format!("@contract/remainder {id}={seller_acct}"));
    ok(tw, buyer, &format!("@contract/condition {id}=deposit #{} {vault}", widget.0));
    ok(tw, buyer, &format!("@contract/offer {id}"));
    ok(tw, buyer, &format!("@contract/accept {id}"));
    ok(tw, seller, &format!("@contract/accept {id}"));
    Deposit { seller, buyer, buyer_acct, seller_acct, widget, vault, id }
}

#[test]
fn a_deposited_object_is_locked_while_the_contract_lives_and_comes_back_on_a_refund() {
    let mut tw = boot();
    let d = deposit_rig(&mut tw);
    assert_eq!(state(&tw, d.id), ContractState::Active);
    assert_eq!(
        snap(&tw).custody_lock(d.widget, d.vault),
        Some(d.id),
        "a live contract's DepositInto did not lock the object"
    );
    // The lock has no exception: not the depositor, not the buyer, not staff.
    for who in [d.seller, d.buyer, GOD] {
        err(&mut tw, who, &format!("@vault/withdraw #{}={}", d.widget.0, d.vault));
    }
    let chest = snap(&tw).vault(d.vault).expect("vault").backing.object().expect("backing");
    assert_eq!(loc(&tw, d.widget), Some(chest), "the locked object moved");
    // The depositor the journal named is on the record.
    let c = contract(&tw, d.id);
    assert_eq!(c.deposits.len(), 1);
    assert_eq!(c.deposits[0].deposited_by, PrincipalId::Object(d.seller));

    // Staff cancel it: the state change lifts the lock, and the refund in the same
    // step releases the escrow AND hands the object back to its depositor.
    ok(&mut tw, GOD, &format!("@contract/cancel {}", d.id));
    assert_eq!(state(&tw, d.id), ContractState::Cancelled);
    assert_eq!(snap(&tw).custody_lock(d.widget, d.vault), None, "the lock survived the state change");
    assert_eq!(held(&tw, d.buyer_acct), 0, "the refund did not release the escrow");
    assert_eq!(bal(&tw, d.buyer_acct), 500, "the refund moved money");
    assert_eq!(bal(&tw, d.seller_acct), 0);
    assert_eq!(
        loc(&tw, d.widget),
        Some(d.seller),
        "the refund did not return the deposit to its depositor"
    );
}

#[test]
fn a_withdrawn_deposit_leaves_the_contract_ready_to_settle_with_nothing_moved() {
    let mut tw = boot();
    let d = deposit_rig(&mut tw);
    // The deposit leaves the vault without the economy's help — a `@teleport`, a
    // ruleset move, anything that is not a custody op. This is exactly the shape live
    // re-verification exists for (D-E4-4): the attestation stood, and the world moved.
    tw.world_mut().move_to(d.widget, d.seller).expect("spirit it out");
    let before = journal_len(&tw);
    sweep_at(&mut tw, 100);
    assert_eq!(journal_len(&tw), before, "a failed re-verification posted something");
    assert_eq!(bal(&tw, d.seller_acct), 0, "a failed re-verification paid the seller");
    assert_eq!(held(&tw, d.buyer_acct), 200, "a failed re-verification released the escrow");
    assert!(
        matches!(state(&tw, d.id), ContractState::Active | ContractState::ReadyToSettle),
        "a re-verification failure changed the state to {:?}",
        state(&tw, d.id)
    );
}

// ======================================================================
// Time: the After delay, expiry, forfeit
// ======================================================================

#[test]
fn an_after_condition_delays_settlement_until_the_sweep() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    ok(&mut tw, r.johnson, "@contract/draft Fence");
    let id = only_contract(&tw);
    ok(&mut tw, r.johnson, &format!("@contract/party {id}=#{}", r.a.0));
    ok(&mut tw, r.johnson, &format!("@contract/pay {id}={} 100", r.j_acct));
    ok(&mut tw, r.johnson, &format!("@contract/split {id}={} 1000‰", r.a_acct));
    ok(&mut tw, r.johnson, &format!("@contract/remainder {id}={}", r.j_acct));
    ok(&mut tw, r.johnson, &format!("@contract/condition {id}=attest party:#{}", r.johnson.0));
    ok(&mut tw, r.johnson, &format!("@contract/condition {id}=after 0 600"));
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
    ok(&mut tw, r.a, &format!("@contract/accept {id}"));

    tw.set_now(1_000);
    ok(&mut tw, r.johnson, &format!("@contract/attest {id}=0"));
    assert_eq!(state(&tw, id), ContractState::Active, "the delay did not hold settlement back");
    assert_eq!(bal(&tw, r.a_acct), 0);

    sweep_at(&mut tw, 1_100);
    assert_eq!(state(&tw, id), ContractState::Active, "the delay expired early");
    sweep_at(&mut tw, 1_700);
    assert!(matches!(state(&tw, id), ContractState::Settled { .. }), "the sweep did not settle it");
    assert_eq!(bal(&tw, r.a_acct), 100);
}

#[test]
fn an_expired_contract_refunds_and_a_forfeiting_one_pays_the_named_account() {
    for forfeit in [false, true] {
        let mut tw = boot();
        let r = run_rig(&mut tw, 3_000);
        ok(&mut tw, r.johnson, "@contract/draft Deadline");
        let id = only_contract(&tw);
        ok(&mut tw, r.johnson, &format!("@contract/party {id}=#{}", r.a.0));
        ok(&mut tw, r.johnson, &format!("@contract/pay {id}={} 250", r.j_acct));
        ok(&mut tw, r.johnson, &format!("@contract/split {id}={} 1000‰", r.a_acct));
        ok(&mut tw, r.johnson, &format!("@contract/remainder {id}={}", r.j_acct));
        // A condition nobody will satisfy, so the deadline is what happens.
        ok(&mut tw, r.johnson, &format!("@contract/condition {id}=attest party:#{}", r.a.0));
        if forfeit {
            ok(&mut tw, r.johnson, &format!("@contract/deadline {id}=600 forfeit={}", r.b_acct));
        } else {
            ok(&mut tw, r.johnson, &format!("@contract/deadline {id}=600 refund"));
        }
        ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
        ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
        ok(&mut tw, r.a, &format!("@contract/accept {id}"));
        assert_eq!(held(&tw, r.j_acct), 250);

        sweep_at(&mut tw, 1_000);
        assert_eq!(state(&tw, id), ContractState::Expired, "the deadline did not lapse it");
        assert_eq!(held(&tw, r.j_acct), 0, "the escrow is still reserved after expiry");
        if forfeit {
            assert_eq!(bal(&tw, r.b_acct), 250, "the forfeit did not pay the named account");
            assert_eq!(bal(&tw, r.j_acct), 3_000 - 250);
        } else {
            assert_eq!(bal(&tw, r.b_acct), 0, "a refund paid somebody");
            assert_eq!(bal(&tw, r.j_acct), 3_000, "the refund did not give the money back");
        }
    }
}

#[test]
fn an_offered_contract_that_lapses_refunds_even_when_its_terms_say_forfeit() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    ok(&mut tw, r.johnson, "@contract/draft Unbound");
    let id = only_contract(&tw);
    ok(&mut tw, r.johnson, &format!("@contract/party {id}=#{}", r.a.0));
    ok(&mut tw, r.johnson, &format!("@contract/pay {id}={} 250", r.j_acct));
    ok(&mut tw, r.johnson, &format!("@contract/split {id}={} 1000‰", r.a_acct));
    ok(&mut tw, r.johnson, &format!("@contract/remainder {id}={}", r.j_acct));
    ok(&mut tw, r.johnson, &format!("@contract/deadline {id}=600 forfeit={}", r.b_acct));
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    // Only the Johnson accepts: the agreement never came into force.
    ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));

    sweep_at(&mut tw, 1_000);
    assert_eq!(state(&tw, id), ContractState::Expired);
    assert_eq!(bal(&tw, r.b_acct), 0, "an unbound contract forfeited somebody's money");
    assert_eq!(bal(&tw, r.j_acct), 3_000, "the money was not given back");
    assert_eq!(held(&tw, r.j_acct), 0);
}

// ======================================================================
// Dispute, agreement, resolution, and the ladder
// ======================================================================

#[test]
fn a_dispute_freezes_the_escrow_and_unanimous_agreement_ends_it() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    let id = offered_run(&mut tw, &r, 300);
    accept_all(&mut tw, &r, id);
    ok(&mut tw, r.a, &format!("@contract/dispute {id}"));
    assert_eq!(state(&tw, id), ContractState::Disputed);

    // Frozen: no release, no capture, and the sweep settles nothing — even once the
    // evidence a dispute was waiting for arrives.
    let hold = contract(&tw, id).escrow[0];
    err(&mut tw, r.johnson, &format!("@hold/release {hold}"));
    ok(&mut tw, r.johnson, &format!("@contract/attest {id}=0"));
    assert_eq!(state(&tw, id), ContractState::Disputed, "an attestation settled a disputed contract");
    sweep_at(&mut tw, 100);
    assert_eq!(state(&tw, id), ContractState::Disputed, "a disputed contract settled");
    assert_eq!(held(&tw, r.j_acct), 300, "a disputed contract's escrow moved");
    assert_eq!(bal(&tw, r.a_acct), 0);

    // Level 0: unanimity resolves it with no staff at all.
    for who in [r.johnson, r.a, r.b] {
        ok(&mut tw, who, &format!("@contract/agree {id}=refund"));
        assert_eq!(state(&tw, id), ContractState::Disputed, "a partial agreement resolved it");
    }
    ok(&mut tw, r.c, &format!("@contract/agree {id}=refund"));
    assert_eq!(state(&tw, id), ContractState::Cancelled, "unanimity did not resolve the dispute");
    assert_eq!(held(&tw, r.j_acct), 0);
    assert_eq!(bal(&tw, r.j_acct), 3_000);
}

#[test]
fn staff_resolve_settles_a_dispute_and_still_re_verifies() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    let id = offered_run(&mut tw, &r, 300);
    accept_all(&mut tw, &r, id);
    ok(&mut tw, r.b, &format!("@contract/dispute {id}"));

    // Unanimity is consent to settle, not evidence: nothing is attested, so a staff
    // resolution re-verifies, refuses, and pays nobody (D-E4-4).
    ok(&mut tw, GOD, &format!("@contract/resolve {id}=settle"));
    assert_ne!(bal(&tw, r.a_acct), 99, "an unmet condition was paid out");
    assert_eq!(held(&tw, r.j_acct), 300, "a refused resolution released the escrow");

    // With the evidence in hand it settles on the ordinary retry path.
    ok(&mut tw, r.johnson, &format!("@contract/attest {id}=0"));
    assert!(matches!(state(&tw, id), ContractState::Settled { .. }));
    assert_eq!(bal(&tw, r.a_acct), 99);
}

#[test]
fn a_long_dispute_pages_once_and_an_acknowledged_alert_reopens_rather_than_re_minting() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    let id = offered_run(&mut tw, &r, 300);
    accept_all(&mut tw, &r, id);
    tw.set_now(1_000);
    ok(&mut tw, r.a, &format!("@contract/dispute {id}"));

    let first: Vec<_> = store(&tw)
        .escalations()
        .filter(|e| e.subject == EscalationSubject::Contract(id))
        .map(|e| e.id)
        .collect();
    assert_eq!(first.len(), 1, "a dispute opened {} records", first.len());
    let alert = first[0];

    // Three days later the sweep pages again — and it is the SAME record.
    sweep_at(&mut tw, 1_000 + 4 * 86_400);
    let after: Vec<_> = store(&tw)
        .escalations()
        .filter(|e| e.subject == EscalationSubject::Contract(id))
        .map(|e| e.id)
        .collect();
    assert_eq!(after, vec![alert], "the ladder minted a second record for one subject");

    // Acknowledge it without fixing the cause; past the cool-off it REOPENS in place.
    ok(&mut tw, GOD, &format!("@econ/ack {alert}"));
    assert!(!store(&tw).escalation(alert).expect("alert").is_open());
    sweep_at(&mut tw, 1_000 + 4 * 86_400 + 2 * 3_600);
    let record = store(&tw).escalation(alert).expect("alert");
    assert!(record.is_open(), "the acknowledged alert did not reopen when its cause persisted");
    assert_eq!(record.reopened, 1, "the reopen tally was not kept");
    assert_eq!(
        store(&tw).escalations().filter(|e| e.subject == EscalationSubject::Contract(id)).count(),
        1,
        "a reopen minted a fresh id"
    );
}

// ======================================================================
// The E6 extensions: outcomes, roster lock, amendment, adjudicator, roles
// ======================================================================

/// A run with a two-entry outcome table and a named adjudicator.
fn outcome_run(tw: &mut TestWorld, r: &Run, judge: ObjId) -> ContractId {
    ok(tw, r.johnson, "@contract/draft Outcome");
    let id = only_contract(tw);
    ok(tw, r.johnson, &format!("@contract/party {id}=#{}", r.a.0));
    ok(tw, r.johnson, &format!("@contract/pay {id}={} 300", r.j_acct));
    // Two mutually exclusive outcome conditions; nothing else gates the contract.
    ok(tw, r.johnson, &format!("@contract/condition {id}=attest party:#{}", r.a.0));
    ok(tw, r.johnson, &format!("@contract/condition {id}=attest party:#{}", r.a.0));
    ok(tw, r.johnson, &format!("@contract/adjudicator {id}=party:#{}", judge.0));
    ok(tw, r.johnson, &format!("@contract/outcome {id}=success when=0 pay={}:300", r.a_acct));
    ok(
        tw,
        r.johnson,
        &format!("@contract/outcome {id}=failure when=1 pay={}:100,{}:200", r.a_acct, r.j_acct),
    );
    // The single split is unused with a table, but a remainder account is still part
    // of a well-formed surface.
    ok(tw, r.johnson, &format!("@contract/remainder {id}={}", r.j_acct));
    ok(tw, r.johnson, &format!("@contract/offer {id}"));
    ok(tw, r.johnson, &format!("@contract/accept {id}"));
    ok(tw, r.a, &format!("@contract/accept {id}"));
    id
}

#[test]
fn the_attested_outcome_is_the_one_that_pays() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    let judge = r.watcher;
    let id = outcome_run(&mut tw, &r, judge);
    assert_eq!(state(&tw, id), ContractState::Active);

    // The failure branch is what the adjudicator attests.
    ok(&mut tw, judge, &format!("@contract/attest {id}=1"));
    assert!(matches!(state(&tw, id), ContractState::Settled { .. }), "the outcome did not settle");
    assert_eq!(bal(&tw, r.a_acct), 100, "the wrong outcome paid");
    assert_eq!(bal(&tw, r.j_acct), 3_000 - 300 + 200);
}

#[test]
fn only_the_named_adjudicator_may_attest_an_adjudicated_condition() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    let judge = r.watcher;
    let id = outcome_run(&mut tw, &r, judge);

    // The condition's own attestor set names Alder — and an adjudicated contract does
    // not let an interested party discharge its own condition.
    let seen = err(&mut tw, r.a, &format!("@contract/attest {id}=0"));
    assert!(seen.contains("adjudicator"), "wrong refusal: {seen}");
    err(&mut tw, r.johnson, &format!("@contract/attest {id}=0"));
    err(&mut tw, GOD, &format!("@contract/attest {id}=0"));
    assert_eq!(state(&tw, id), ContractState::Active, "somebody else's attestation settled it");
    assert_eq!(bal(&tw, r.a_acct), 0);

    ok(&mut tw, judge, &format!("@contract/attest {id}=0"));
    assert!(matches!(state(&tw, id), ContractState::Settled { .. }));
    assert_eq!(bal(&tw, r.a_acct), 300, "the success outcome did not pay");
}

#[test]
fn two_satisfied_outcomes_are_a_dispute_and_pay_nobody() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    let judge = r.watcher;
    // Two outcomes gated on the SAME condition: whatever the adjudicator attests,
    // the contract does not say which result happened.
    ok(&mut tw, r.johnson, "@contract/draft Ambiguous");
    let id = only_contract(&tw);
    ok(&mut tw, r.johnson, &format!("@contract/party {id}=#{}", r.a.0));
    ok(&mut tw, r.johnson, &format!("@contract/pay {id}={} 300", r.j_acct));
    ok(&mut tw, r.johnson, &format!("@contract/condition {id}=attest party:#{}", judge.0));
    ok(&mut tw, r.johnson, &format!("@contract/outcome {id}=paid when=0 pay={}:300", r.a_acct));
    ok(&mut tw, r.johnson, &format!("@contract/outcome {id}=kept when=0 pay={}:300", r.j_acct));
    ok(&mut tw, r.johnson, &format!("@contract/remainder {id}={}", r.j_acct));
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
    ok(&mut tw, r.a, &format!("@contract/accept {id}"));

    let before = journal_len(&tw);
    ok(&mut tw, judge, &format!("@contract/attest {id}=0"));
    // §3.5: ambiguity is a dispute, at once — no retry can make a contract that does
    // not say what happened say it, so the attempts are not spent waiting.
    assert_eq!(state(&tw, id), ContractState::Disputed, "an ambiguous outcome did not dispute");
    assert_eq!(journal_len(&tw), before, "an ambiguous outcome posted something");
    assert_eq!(bal(&tw, r.a_acct), 0, "an ambiguous outcome paid a payee");
    assert_eq!(bal(&tw, r.j_acct), 3_000, "an ambiguous outcome moved money");
    assert_eq!(held(&tw, r.j_acct), 300, "an ambiguous outcome released the escrow");
    assert!(
        store(&tw).escalations().any(|e| e.subject == EscalationSubject::Contract(id)),
        "an ambiguous outcome paged nobody"
    );
}

#[test]
fn the_roster_locks_on_the_first_acceptance_and_refuses_a_late_party() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    let id = offered_run(&mut tw, &r, 300);
    // The issuer deliberately re-opens the roster (08e answers 5/7's slot shape).
    ok(&mut tw, r.johnson, &format!("@contract/roster {id}=open"));
    ok(&mut tw, r.johnson, &format!("@contract/party {id}=#{}", r.watcher.0));
    assert_eq!(contract(&tw, id).consent.parties.len(), 5);

    // The first acceptance freezes it, and a late party is refused.
    ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
    assert!(contract(&tw, id).roster_locked);
    let extra = tw.spawn_player("Late");
    let seen = err(&mut tw, r.johnson, &format!("@contract/party {id}=#{}", extra.0));
    assert!(seen.contains("locked"), "wrong refusal: {seen}");
    err(&mut tw, r.johnson, &format!("@contract/roster {id}=open"));
    assert_eq!(contract(&tw, id).consent.parties.len(), 5, "a late party joined a locked roster");
}

#[test]
fn an_amendment_refunds_every_acceptance_and_re_collects_them() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    let id = offered_run(&mut tw, &r, 300);
    ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
    ok(&mut tw, r.a, &format!("@contract/accept {id}"));
    assert_eq!(held(&tw, r.j_acct), 300);

    ok(&mut tw, r.johnson, &format!("@contract/amend {id}"));
    let c = contract(&tw, id);
    assert_eq!(c.state, ContractState::Draft, "an amendment did not re-open the surface");
    assert!(c.acceptances.is_empty(), "an amendment kept an acceptance");
    assert!(!c.roster_locked, "an amendment kept the roster locked");
    assert_eq!(held(&tw, r.j_acct), 0, "an amendment kept the escrow");
    assert_eq!(bal(&tw, r.j_acct), 3_000, "an amendment moved money");

    // Amend the shares, re-offer, re-collect, and settle on the NEW terms.
    ok(&mut tw, r.johnson, &format!("@contract/split {id}={} 1000‰", r.a_acct));
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    accept_all(&mut tw, &r, id);
    ok(&mut tw, r.johnson, &format!("@contract/attest {id}=0"));
    assert_eq!(bal(&tw, r.a_acct), 300, "the amended split did not take effect");
    assert_eq!(bal(&tw, r.b_acct), 0);
}

#[test]
fn an_amended_contract_refunds_again_instead_of_hitting_its_first_refunds_key() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    let id = offered_run(&mut tw, &r, 300);
    ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
    ok(&mut tw, r.johnson, &format!("@contract/amend {id}"));
    assert_eq!(contract(&tw, id).amendments, 1, "the amendment did not advance the key generation");
    assert_eq!(held(&tw, r.j_acct), 0);

    // The SAME contract id, offered and funded again, and then cancelled. Its refund
    // must actually run: at generation 0 the key would resolve to the amendment's own
    // refund as an idempotency HIT and hand nothing back.
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
    assert_eq!(held(&tw, r.j_acct), 300);
    ok(&mut tw, r.johnson, &format!("@contract/cancel {id}"));
    assert_eq!(held(&tw, r.j_acct), 0, "the second refund hit the first refund's key");
    assert_eq!(bal(&tw, r.j_acct), 3_000);

    // Generation 0 is byte-identical to what E4a specified, so every stored record
    // keeps resolving; only the amended generations move.
    assert_eq!(mush_economy::contract_op_key(id, "refund", 0), mush_economy::refund_key(id));
    assert_ne!(mush_economy::contract_op_key(id, "refund", 1), mush_economy::refund_key(id));
}

#[test]
fn the_five_roles_are_recorded_separately_and_none_of_them_is_authority() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    let author = tw.spawn_player("Author");
    let judge = r.watcher;
    ok(&mut tw, r.johnson, "@contract/draft Roles");
    let id = only_contract(&tw);
    ok(&mut tw, r.johnson, &format!("@contract/party {id}=#{}", r.a.0));
    ok(&mut tw, r.johnson, &format!("@contract/pay {id}={} 100", r.j_acct));
    ok(&mut tw, r.johnson, &format!("@contract/split {id}={} 1000‰", r.a_acct));
    ok(&mut tw, r.johnson, &format!("@contract/remainder {id}={}", r.j_acct));
    ok(&mut tw, r.johnson, &format!("@contract/condition {id}=attest party:#{}", r.a.0));
    ok(&mut tw, r.johnson, &format!("@contract/adjudicator {id}=party:#{}", judge.0));
    ok(&mut tw, r.johnson, &format!("@contract/role {id}=author #{}", author.0));
    ok(&mut tw, r.johnson, &format!("@contract/role {id}=johnson #{}", r.johnson.0));
    ok(&mut tw, r.johnson, &format!("@contract/role {id}=facilitator #{}", judge.0));
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));

    let c = contract(&tw, id);
    assert_eq!(c.roles.author, Some(PrincipalId::Object(author)));
    assert_eq!(c.roles.payer, Some(PrincipalId::Object(r.johnson)), "the payer is the account's owner");
    assert_eq!(c.roles.representative, Some(PrincipalId::Object(r.johnson)));
    assert_eq!(c.roles.facilitator, Some(PrincipalId::Object(judge)));

    // The author is not a party: being named a role authorizes nothing at all.
    let seen = err(&mut tw, author, &format!("@contract/accept {id}"));
    assert!(seen.contains("No such contract."), "wrong refusal: {seen}");
    ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
    ok(&mut tw, r.a, &format!("@contract/accept {id}"));
    // The adjudicator is a role too, and the only one that can attest.
    err(&mut tw, r.a, &format!("@contract/attest {id}=0"));
    ok(&mut tw, judge, &format!("@contract/attest {id}=0"));
    assert_eq!(bal(&tw, r.a_acct), 100);
    assert_eq!(bal(&tw, r.j_acct), 3_000 - 100, "the payer's account is what funded it");
}

// ======================================================================
// Visibility and hidden parties (§4.5, doc 10 T6)
// ======================================================================

#[test]
fn a_stranger_gets_the_nonexistent_text_for_every_contract_verb() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    let id = offered_run(&mut tw, &r, 300);
    let stranger = tw.spawn_player("Nobody");

    for line in [
        format!("@contract/show {id}"),
        format!("@contract/accept {id}"),
        format!("@contract/attest {id}=0"),
        format!("@contract/dispute {id}"),
        format!("@contract/cancel {id}"),
        format!("@contract/agree {id}=refund"),
        format!("@contract/resolve {id}=refund"),
        format!("@contract/offer {id}"),
        format!("@contract/party {id}=#{}", stranger.0),
    ] {
        let seen = err(&mut tw, stranger, &line);
        assert!(seen.contains("No such contract."), "`{line}` leaked: {seen}");
    }
    // ...and it is not discoverable through the list either.
    let seen = ok(&mut tw, stranger, "@contract/list");
    assert!(seen.contains("No contracts."), "the list leaked a contract: {seen}");
    // A contract id that does not exist at all renders identically.
    let seen = err(&mut tw, stranger, "@contract/show 4242");
    assert!(seen.contains("No such contract."), "{seen}");
}

#[test]
fn an_audience_sees_a_declared_operation_and_a_stranger_does_not() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    let stranger = tw.spawn_player("Nobody");
    ok(&mut tw, r.johnson, "@contract/draft Operation");
    let id = only_contract(&tw);
    ok(&mut tw, r.johnson, &format!("@contract/party {id}=#{}", r.a.0));
    ok(&mut tw, r.johnson, &format!("@contract/pay {id}={} 100", r.j_acct));
    ok(&mut tw, r.johnson, &format!("@contract/split {id}={} 1000‰", r.a_acct));
    ok(&mut tw, r.johnson, &format!("@contract/remainder {id}={}", r.j_acct));
    ok(&mut tw, r.johnson, &format!("@contract/visibility {id}=audience #{}", r.watcher.0));
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));

    // T6: visible to the eligible defenders, without being published.
    let seen = ok(&mut tw, r.watcher, &format!("@contract/show {id}"));
    assert!(seen.contains("Operation"), "the audience could not read it: {seen}");
    assert!(seen.contains("audience"), "the visibility class is not shown: {seen}");
    let seen = err(&mut tw, stranger, &format!("@contract/show {id}"));
    assert!(seen.contains("No such contract."), "an audience contract leaked: {seen}");
    // An audience contract is emphatically NOT public.
    assert!(!contract(&tw, id).consent.visibility.is_public());

    // The audience is canonicalized by principal key inside the consent hash, so
    // re-typing it in another order is the SAME surface.
    let hash = contract(&tw, id).consent_hash;
    let mut c = contract(&tw, id);
    c.consent.visibility = mush_economy::EconomicVisibility::Audience {
        members: vec![PrincipalId::Object(r.watcher)],
    };
    assert_eq!(mush_economy::consent_hash(&c.issuer, &c.consent), hash);
}

#[test]
fn a_public_contract_is_a_disclosure_every_party_accepts() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    let stranger = tw.spawn_player("Nobody");
    ok(&mut tw, r.johnson, "@contract/draft Open");
    let id = only_contract(&tw);
    ok(&mut tw, r.johnson, &format!("@contract/party {id}=#{}", r.a.0));
    ok(&mut tw, r.johnson, &format!("@contract/pay {id}={} 100", r.j_acct));
    ok(&mut tw, r.johnson, &format!("@contract/split {id}={} 1000‰", r.a_acct));
    ok(&mut tw, r.johnson, &format!("@contract/remainder {id}={}", r.j_acct));
    ok(&mut tw, r.johnson, &format!("@contract/visibility {id}=public"));
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));

    // Anyone may read it...
    ok(&mut tw, stranger, &format!("@contract/show {id}"));
    // ...and every party's acceptance binds to a surface that SAYS so: flipping the
    // visibility after Offer is refused, exactly like any other mutation.
    ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
    err(&mut tw, r.johnson, &format!("@contract/visibility {id}=parties"));
    let c = contract(&tw, id);
    assert!(c.consent.visibility.is_public());
    assert!(c.accepted_by(&PrincipalId::Object(r.johnson)));
    // A party reads the other parties as the surface names them, and no balances.
    let seen = ok(&mut tw, r.a, &format!("@contract/show {id}"));
    assert!(seen.contains("parties:"), "{seen}");
    assert!(!seen.contains("posted"), "a contract view leaked a balance: {seen}");
}

// ======================================================================
// Leases: the encumbrance and the obligation
// ======================================================================

#[test]
fn a_lease_encumbers_the_pledge_and_creates_its_rent_at_activation() {
    let mut tw = boot();
    let landlord = tw.spawn_player("Landlord");
    let tenant = tw.spawn_player("Tenant");
    ok(&mut tw, GOD, "@instrument/register gold=0");
    ok(&mut tw, GOD, "@account/system faucet=gold");
    ok(&mut tw, GOD, &format!("@account/open #{}=gold", landlord.0));
    ok(&mut tw, GOD, &format!("@account/open #{}=gold", tenant.0));
    let faucet = snap(&tw)
        .accounts()
        .find(|x| matches!(x.owner, PrincipalId::System(_)))
        .expect("faucet")
        .id;
    let l_acct = account_of(&tw, landlord);
    let t_acct = account_of(&tw, tenant);
    ok(&mut tw, GOD, &format!("@econ/mint {faucet}={t_acct} 1000"));
    let flat = thing(&mut tw, "flat");
    ok(&mut tw, GOD, &format!("@asset/title #{}=#{}", flat.0, landlord.0));

    ok(&mut tw, landlord, "@contract/draft Lease");
    let id = only_contract(&tw);
    ok(&mut tw, landlord, &format!("@contract/party {id}=#{}", tenant.0));
    ok(&mut tw, landlord, &format!("@contract/pay {id}={t_acct} 100"));
    ok(&mut tw, landlord, &format!("@contract/split {id}={l_acct} 1000‰"));
    ok(&mut tw, landlord, &format!("@contract/remainder {id}={l_acct}"));
    ok(&mut tw, landlord, &format!("@contract/deliver {id}=collateral #{} #{}", flat.0, landlord.0));
    ok(
        &mut tw,
        landlord,
        &format!("@contract/deliver {id}=obligation {t_acct} {l_acct} 50 every=600"),
    );
    ok(&mut tw, landlord, &format!("@contract/condition {id}=attest party:#{}", landlord.0));
    ok(&mut tw, landlord, &format!("@contract/offer {id}"));
    ok(&mut tw, landlord, &format!("@contract/accept {id}"));
    ok(&mut tw, tenant, &format!("@contract/accept {id}"));
    assert_eq!(state(&tw, id), ContractState::Active);

    // A contract that also creates an obligation pledges its collateral as a LEASE.
    let asset = snap(&tw).asset(flat).expect("asset");
    assert_eq!(
        asset.encumbrances,
        vec![Encumbrance::Lease { lessor: PrincipalId::Object(landlord), contract: id }],
        "activation did not place the lease claim"
    );
    let rent = snap(&tw)
        .obligations()
        .find(|o| o.cause == ObligationCause::Contract(id))
        .expect("the lease's rent")
        .clone();
    assert_eq!(rent.amount.minor(), 50);
    assert_eq!(rent.status, ObligationStatus::Active);

    // Staff end it: the rent stops and the claim comes off.
    ok(&mut tw, GOD, &format!("@contract/cancel {id}"));
    assert_eq!(
        snap(&tw).obligation(rent.id).expect("rent").status,
        ObligationStatus::Cancelled,
        "the lease's rent outlived the lease"
    );
    assert!(
        snap(&tw).asset(flat).expect("asset").encumbrances.is_empty(),
        "the lease claim stranded on the asset"
    );
    assert_eq!(held(&tw, t_acct), 0, "the refund did not release the escrow");
}

#[test]
fn a_pledge_that_was_never_placed_does_not_block_the_refund_that_ends_the_offer() {
    let mut tw = boot();
    let landlord = tw.spawn_player("Landlord");
    let tenant = tw.spawn_player("Tenant");
    ok(&mut tw, GOD, "@instrument/register gold=0");
    ok(&mut tw, GOD, "@account/system faucet=gold");
    ok(&mut tw, GOD, &format!("@account/open #{}=gold", landlord.0));
    ok(&mut tw, GOD, &format!("@account/open #{}=gold", tenant.0));
    let faucet = snap(&tw)
        .accounts()
        .find(|x| matches!(x.owner, PrincipalId::System(_)))
        .expect("faucet")
        .id;
    let l_acct = account_of(&tw, landlord);
    let t_acct = account_of(&tw, tenant);
    ok(&mut tw, GOD, &format!("@econ/mint {faucet}={t_acct} 1000"));
    let flat = thing(&mut tw, "flat");
    ok(&mut tw, GOD, &format!("@asset/title #{}=#{}", flat.0, landlord.0));

    ok(&mut tw, landlord, "@contract/draft Pledge");
    let id = only_contract(&tw);
    ok(&mut tw, landlord, &format!("@contract/party {id}=#{}", tenant.0));
    ok(&mut tw, landlord, &format!("@contract/pay {id}={t_acct} 100"));
    ok(&mut tw, landlord, &format!("@contract/split {id}={l_acct} 1000‰"));
    ok(&mut tw, landlord, &format!("@contract/remainder {id}={l_acct}"));
    ok(&mut tw, landlord, &format!("@contract/deliver {id}=collateral #{} #{}", flat.0, landlord.0));
    ok(&mut tw, landlord, &format!("@contract/offer {id}"));
    // Only the tenant accepts, so the claim is never placed — activation is what
    // places it. Cancelling now must still hand the escrow back: a `Release` of an
    // encumbrance the asset does not carry would fail the whole refund plan.
    ok(&mut tw, tenant, &format!("@contract/accept {id}"));
    assert_eq!(held(&tw, t_acct), 100);
    assert!(snap(&tw).asset(flat).expect("asset").encumbrances.is_empty());

    ok(&mut tw, landlord, &format!("@contract/cancel {id}"));
    assert_eq!(state(&tw, id), ContractState::Cancelled);
    assert_eq!(held(&tw, t_acct), 0, "the refund stranded the escrow on an unplaced pledge");
    assert_eq!(bal(&tw, t_acct), 1_000);
}

// ======================================================================
// Boot: reconciliation at every state
// ======================================================================

#[test]
fn every_contract_state_survives_a_reboot() {
    // Draft / Offered / Active / Disputed / Settled / Cancelled, each driven to that
    // state through the real commands and then replayed through the real boot.
    for target in ["draft", "offered", "active", "disputed", "settled", "cancelled"] {
        let mut tw = boot();
        let r = run_rig(&mut tw, 3_000);
        let id = if target == "draft" {
            ok(&mut tw, r.johnson, "@contract/draft Reboot");
            only_contract(&tw)
        } else {
            let id = offered_run(&mut tw, &r, 300);
            match target {
                "offered" => {}
                _ => accept_all(&mut tw, &r, id),
            }
            match target {
                "disputed" => ok(&mut tw, r.a, &format!("@contract/dispute {id}")),
                "settled" => ok(&mut tw, r.johnson, &format!("@contract/attest {id}=0")),
                "cancelled" => ok(&mut tw, GOD, &format!("@contract/cancel {id}")),
                _ => String::new(),
            };
            id
        };
        let before = state(&tw, id);
        let held_before = held(&tw, r.j_acct);
        let bal_before = bal(&tw, r.j_acct);

        let kv = fold_kv(&mut tw);
        reboot(&mut tw, kv);
        assert!(!store(&tw).is_frozen(), "{target}: a clean reboot froze the economy");
        assert_eq!(state(&tw, id), before, "{target}: the state did not survive the reboot");
        assert_eq!(held(&tw, r.j_acct), held_before, "{target}: the escrow changed across a reboot");
        assert_eq!(bal(&tw, r.j_acct), bal_before, "{target}: a balance changed across a reboot");
    }
}

#[test]
fn a_live_contract_whose_escrow_is_gone_comes_back_disputed() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    let id = offered_run(&mut tw, &r, 300);
    accept_all(&mut tw, &r, id);
    let mut kv = fold_kv(&mut tw);
    // A record claiming an escrow hold the journal never placed: an agreement
    // standing on money that is not there.
    patch(&mut kv, &format!("contract:{id}"), |v| {
        v["escrow"] = serde_json::json!([9_999]);
    });
    reboot(&mut tw, kv);
    assert_eq!(state(&tw, id), ContractState::Disputed, "boot did not quarantine a broken escrow");
    assert!(!store(&tw).is_frozen(), "one broken contract froze the whole economy");
    assert!(
        store(&tw).escalations().any(|e| e.subject == EscalationSubject::Contract(id)),
        "boot quarantined a contract without paging"
    );
}

#[test]
fn a_settled_contract_the_journal_does_not_know_freezes_the_economy() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    let id = offered_run(&mut tw, &r, 300);
    accept_all(&mut tw, &r, id);
    ok(&mut tw, r.johnson, &format!("@contract/attest {id}=0"));
    let mut kv = fold_kv(&mut tw);
    patch(&mut kv, &format!("contract:{id}"), |v| {
        v["state"] = serde_json::json!({ "Settled": { "transaction": 9_999 } });
    });
    reboot(&mut tw, kv);
    assert!(
        store(&tw).is_frozen(),
        "a settlement the journal does not know did not freeze the economy"
    );
    assert!(
        store(&tw)
            .escalations()
            .any(|e| e.subject == EscalationSubject::Contract(id)),
        "the freeze opened no companion record"
    );
    // Every account is frozen with it, so nothing can be spent on a journal nobody
    // has audited.
    assert_eq!(snap(&tw).account(r.j_acct).expect("account").status, AccountStatus::Frozen);
}

#[test]
fn the_contract_allocator_is_floored_past_every_key_and_meta() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    ok(&mut tw, r.johnson, "@contract/draft One");
    ok(&mut tw, r.johnson, "@contract/draft Two");
    let mut kv = fold_kv(&mut tw);
    // A stale allocator plus a surviving key: the floor must come from the key.
    patch(&mut kv, "meta", |v| {
        v["next_contract_id"] = serde_json::json!(0);
    });
    reboot(&mut tw, kv);
    ok(&mut tw, r.johnson, "@contract/draft Three");
    let ids: Vec<u64> = snap(&tw).contracts().map(|c| c.id.0).collect();
    assert_eq!(ids, vec![0, 1, 2], "the contract allocator re-minted a live id: {ids:?}");
}

// ======================================================================
// Caps and garbage collection
// ======================================================================

#[test]
fn an_issuer_is_capped_at_sixteen_open_contracts_and_a_draft_is_collected() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    tw.set_now(1_000);
    for n in 0..mush_economy::MAX_OPEN_CONTRACTS_PER_ISSUER {
        ok(&mut tw, r.johnson, &format!("@contract/draft Draft{n}"));
    }
    let seen = err(&mut tw, r.johnson, "@contract/draft Overflow");
    assert!(seen.contains("cap is"), "wrong refusal: {seen}");
    assert_eq!(snap(&tw).contracts().count(), mush_economy::MAX_OPEN_CONTRACTS_PER_ISSUER);

    // Eight days later the sweep collects every abandoned draft, and the issuer can
    // open new ones again.
    sweep_at(&mut tw, 1_000 + 8 * 86_400);
    assert_eq!(snap(&tw).contracts().count(), 0, "abandoned drafts were not collected");
    ok(&mut tw, r.johnson, "@contract/draft Fresh");
    // ...and the collected ids are never re-minted.
    let fresh = only_contract(&tw);
    assert!(
        fresh.0 >= u64::try_from(mush_economy::MAX_OPEN_CONTRACTS_PER_ISSUER).expect("small"),
        "a collected contract id was re-minted: {fresh}"
    );
}

#[test]
fn an_offer_refuses_a_surface_that_could_never_settle() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    ok(&mut tw, r.johnson, "@contract/draft Broken");
    let id = only_contract(&tw);
    ok(&mut tw, r.johnson, &format!("@contract/party {id}=#{}", r.a.0));
    ok(&mut tw, r.johnson, &format!("@contract/pay {id}={} 100", r.j_acct));
    // No remainder account at all: a floored split would have nowhere to land.
    ok(&mut tw, r.johnson, &format!("@contract/split {id}={} 500‰", r.a_acct));
    err(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    assert_eq!(state(&tw, id), ContractState::Draft);

    ok(&mut tw, r.johnson, &format!("@contract/remainder {id}={}", r.j_acct));
    // A split that pays out more than is escrowed is infeasible at Offer.
    ok(&mut tw, r.johnson, &format!("@contract/split {id}={} 500", r.a_acct));
    err(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    ok(&mut tw, r.johnson, &format!("@contract/split {id}={} 60", r.a_acct));
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    assert_eq!(state(&tw, id), ContractState::Offered);
}

#[test]
fn the_permitted_evidence_list_is_exhaustive_when_it_is_set() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    ok(&mut tw, r.johnson, "@contract/draft Evidence");
    let id = only_contract(&tw);
    ok(&mut tw, r.johnson, &format!("@contract/party {id}=#{}", r.a.0));
    ok(&mut tw, r.johnson, &format!("@contract/pay {id}={} 100", r.j_acct));
    ok(&mut tw, r.johnson, &format!("@contract/split {id}={} 1000‰", r.a_acct));
    ok(&mut tw, r.johnson, &format!("@contract/remainder {id}={}", r.j_acct));
    ok(&mut tw, r.johnson, &format!("@contract/condition {id}=before 600"));
    ok(&mut tw, r.johnson, &format!("@contract/evidence {id}=attest"));
    // The surface says attestations only, and it carries a deadline condition.
    err(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    ok(&mut tw, r.johnson, &format!("@contract/evidence {id}=attest,before"));
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
}

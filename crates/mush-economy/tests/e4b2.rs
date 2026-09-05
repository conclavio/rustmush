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
fn store(tw: &TestWorld) -> &EconomyStore {
    tw.world()
        .module_state::<EconomyStore>()
        .expect("economy store")
}
fn snap(tw: &TestWorld) -> &EconomySnapshot {
    store(tw).snapshot()
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
    tw.world_mut()
        .create(ObjKind::Thing, name, GOD, Some(LIMBO))
        .expect("create")
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
        ModuleBootData {
            kv,
            streams: HashMap::new(),
        },
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
    ok(
        tw,
        r.johnson,
        &format!("@contract/pay {id}={} {pay}", r.j_acct),
    );
    ok(
        tw,
        r.johnson,
        &format!(
            "@contract/split {id}={} 333‰ {} 333‰ {} 333‰",
            r.a_acct, r.b_acct, r.c_acct
        ),
    );
    ok(
        tw,
        r.johnson,
        &format!("@contract/remainder {id}={}", r.j_acct),
    );
    ok(
        tw,
        r.johnson,
        &format!("@contract/condition {id}=attest party:#{}", r.johnson.0),
    );
    ok(tw, r.johnson, &format!("@contract/offer {id}"));
    id
}

fn accept_all(tw: &mut TestWorld, r: &Run, id: ContractId) {
    for who in [r.johnson, r.a, r.b, r.c] {
        ok(tw, who, &format!("@contract/accept {id}"));
    }
}

/// The rig's faucet account.
fn faucet(tw: &TestWorld) -> AccountId {
    snap(tw)
        .accounts()
        .find(|x| matches!(x.owner, PrincipalId::System(_)))
        .expect("faucet")
        .id
}

/// The **fixer-issuer** shape (E6a step 1): the Johnson drafts and offers, but the
/// PAYER is Alder, who adds her own consideration to the draft (A1's
/// self-consideration) and is minted 500 to fund it. Bex is paid on settlement.
/// Under A′ an amendment by the Johnson voids and refunds Alder's acceptance — he is
/// not the party whose escrow stands.
fn fixer_issued_run(tw: &mut TestWorld, r: &Run, pay: i64) -> ContractId {
    let faucet = faucet(tw);
    ok(tw, GOD, &format!("@econ/mint {faucet}={} 500", r.a_acct));
    ok(tw, r.johnson, "@contract/draft Brokered");
    let id = only_contract(tw);
    for who in [r.a, r.b] {
        ok(tw, r.johnson, &format!("@contract/party {id}=#{}", who.0));
    }
    ok(tw, r.a, &format!("@contract/pay {id}={} {pay}", r.a_acct));
    ok(
        tw,
        r.johnson,
        &format!("@contract/split {id}={} 1000‰", r.b_acct),
    );
    ok(
        tw,
        r.johnson,
        &format!("@contract/remainder {id}={}", r.a_acct),
    );
    ok(
        tw,
        r.johnson,
        &format!("@contract/condition {id}=attest party:#{}", r.johnson.0),
    );
    ok(tw, r.johnson, &format!("@contract/offer {id}"));
    id
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
    assert_eq!(
        held(&tw, r.j_acct),
        0,
        "escrow was reserved before anybody accepted"
    );
    ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
    assert_eq!(
        held(&tw, r.j_acct),
        300,
        "the accept did not place the consideration"
    );
    assert_eq!(bal(&tw, r.j_acct), 3_000, "a reservation moved money");
    assert_eq!(
        state(&tw, id),
        ContractState::Offered,
        "one acceptance activated it"
    );

    for who in [r.a, r.b, r.c] {
        ok(&mut tw, who, &format!("@contract/accept {id}"));
    }
    assert_eq!(
        state(&tw, id),
        ContractState::Active,
        "the last acceptance did not activate it"
    );

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
    assert_eq!(
        bal(&tw, r.j_acct),
        3_000 - 300 + 3,
        "the remainder did not land where declared"
    );
    assert_eq!(
        held(&tw, r.j_acct),
        0,
        "the escrow is still reserved after settlement"
    );
    assert_eq!(
        journal_len(&tw),
        before + 1,
        "settlement was not one transaction"
    );

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
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/party {id}=#{}", r.a.0),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/pay {id}={} 300", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/split {id}={} 1000‰", r.a_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/remainder {id}={}", r.j_acct),
    );
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
            spent: std::collections::BTreeMap::new(),
        };
        mush_economy::settlement_plan(&c, s, &env).expect("a condition-less contract settles")
    };
    let live = snap(&tw).hold(holds[0]).expect("escrow").revision;
    assert_eq!(
        plan.expected_hold_revisions,
        vec![mush_economy::HoldRevision {
            hold: holds[0],
            revision: live
        }],
        "the settlement plan carries no staleness guard over its escrow"
    );
    assert_eq!(plan.idempotency, mush_economy::settle_key(id));

    sweep_at(&mut tw, 100);
    let ContractState::Settled { transaction } = state(&tw, id) else {
        panic!("the sweep did not settle a ready contract")
    };
    let record = snap(&tw)
        .idempotency(&mush_economy::settle_key(id))
        .expect("the deterministic settle key");
    assert_eq!(
        record.transaction, transaction,
        "the settlement did not use contract/{{id}}/settle"
    );
    let tx = store(&tw)
        .transaction(transaction)
        .expect("the settlement transaction");
    assert_eq!(
        tx.hold_effects.len(),
        1,
        "the settlement captured the wrong number of holds"
    );
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
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/party {id}=#{}", r.a.0),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/pay {id}={} 300", r.j_acct),
    );
    // A1: the Johnson cannot name Alder's wallet (he may not view it); Alder adds her
    // own consideration to the draft instead.
    ok(
        &mut tw,
        r.a,
        &format!("@contract/pay {id}={} 200", r.a_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/split {id}={} 1000‰", r.b_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/remainder {id}={}", r.j_acct),
    );
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));

    assert_eq!(state(&tw, id), ContractState::Offered);
    assert_eq!(
        held(&tw, r.a_acct),
        0,
        "a party who never accepted had funds reserved"
    );
    assert_eq!(
        bal(&tw, r.a_acct),
        500,
        "a party who never accepted was debited"
    );
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
    assert_eq!(
        contract(&tw, id).consent_hash,
        hash,
        "the frozen consent hash moved"
    );
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
    ok(
        tw,
        GOD,
        &format!("@account/open org:Ironhold/{}=gold", scope.0),
    );
    ok(tw, GOD, &format!("@account/open #{}=gold", payee.0));
    let acct = snap(tw)
        .accounts()
        .find(|a| a.scope == Some(scope))
        .expect("org account")
        .id;
    let payee_acct = account_of(tw, payee);
    let faucet = snap(tw)
        .accounts()
        .find(|x| matches!(x.owner, PrincipalId::System(_)))
        .expect("faucet")
        .id;
    ok(tw, GOD, &format!("@econ/mint {faucet}={acct} 1000"));
    Org {
        org,
        scope,
        acct,
        member,
        payee,
        payee_acct,
    }
}

#[test]
fn contract_accept_needs_the_capability_and_debit_authority_independently() {
    let mut tw = boot();
    let o = org_rig(&mut tw);
    // Staff drafts an ORG contract on the compartment, so the org is the party.
    ok(
        &mut tw,
        GOD,
        &format!("@contract/draft Supply scope=#{}", o.scope.0),
    );
    let id = only_contract(&tw);
    ok(
        &mut tw,
        GOD,
        &format!("@contract/party {id}=#{}", o.payee.0),
    );
    ok(&mut tw, GOD, &format!("@contract/pay {id}={} 400", o.acct));
    ok(
        &mut tw,
        GOD,
        &format!("@contract/split {id}={} 1000‰", o.payee_acct),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@contract/remainder {id}={}", o.acct),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@contract/condition {id}=attest staff"),
    );
    ok(&mut tw, GOD, &format!("@contract/offer {id}"));

    // No capability at all: the contract does not exist as far as Quinn is concerned.
    let seen = err(&mut tw, o.member, &format!("@contract/accept {id}"));
    assert!(seen.contains("No such contract."), "wrong refusal: {seen}");

    // `contract.accept` alone lets her SEE it and try — and she still cannot escrow
    // the org's money, because that is a separate right (D-E4-4).
    ok(
        &mut tw,
        GOD,
        &format!(
            "@org/grant #{}=Ironhold contract.accept scope=#{}",
            o.member.0, o.scope.0
        ),
    );
    err(&mut tw, o.member, &format!("@contract/accept {id}"));
    assert_eq!(
        held(&tw, o.acct),
        0,
        "an accept with no debit authority escrowed org funds"
    );
    assert!(
        contract(&tw, id).acceptances.is_empty(),
        "a refused accept recorded an acceptance"
    );

    // Both rights: now it works.
    ok(
        &mut tw,
        GOD,
        &format!(
            "@org/grant #{}=Ironhold economy.account.post scope=#{}",
            o.member.0, o.scope.0
        ),
    );
    ok(&mut tw, o.member, &format!("@contract/accept {id}"));
    assert_eq!(
        held(&tw, o.acct),
        400,
        "the accept did not place the org's consideration"
    );
    assert_eq!(contract(&tw, id).acceptances.len(), 1);
    let _ = o.org;
}

#[test]
fn an_approval_rule_on_the_payers_account_records_no_acceptance() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    // D-E4-11: a placement is a debit-authority plan, so the account's rule applies —
    // and an accept that needs approval leaves the contract exactly where it was.
    ok(
        &mut tw,
        GOD,
        &format!("@account/approval {}=10 named:1:#{}", r.j_acct, r.watcher.0),
    );
    let id = offered_run(&mut tw, &r, 300);
    let seen = ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
    assert!(seen.contains("Approval required"), "wrong outcome: {seen}");
    assert!(
        contract(&tw, id).acceptances.is_empty(),
        "an unapproved accept was recorded"
    );
    assert_eq!(
        held(&tw, r.j_acct),
        0,
        "an unapproved accept reserved funds"
    );
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
    // A credit only to a declared payee, and only the EXACT amount the consented
    // split owes them (E4b2 verify fold, M4): 333‰ of the 300 escrowed is 99.
    assert!(settling.authorize(&actor, payee, Amount::from_minor(99)));
    assert!(!settling.authorize(&actor, payee, Amount::from_minor(1)));
    assert!(!settling.authorize(&actor, payee, Amount::from_minor(300)));
    assert!(!settling.authorize(&actor, stranger, Amount::from_minor(1)));
    // The escrow hold: a settlement captures and may not release.
    let hold = s.hold(c.escrow[0]).expect("escrow");
    assert!(settling.authorize_hold(
        &actor,
        hold,
        &mush_economy::HoldOp::Capture {
            hold: hold.id,
            amount: hold.amount,
        }
    ));
    assert!(!settling.authorize_hold(
        &actor,
        hold,
        &mush_economy::HoldOp::Release { hold: hold.id }
    ));
    // ...and an unwind is the other way round.
    let unwinding = ContractAuth::unwinding(&c, s);
    assert!(unwinding.authorize_hold(
        &actor,
        hold,
        &mush_economy::HoldOp::Release { hold: hold.id }
    ));
    assert!(!unwinding.authorize_hold(
        &actor,
        hold,
        &mush_economy::HoldOp::Capture {
            hold: hold.id,
            amount: hold.amount,
        }
    ));
    assert!(
        !unwinding.authorize(&actor, payee, Amount::from_minor(1)),
        "an unwind credited somebody"
    );
    // A capture over the hold's amount is refused whatever the mode.
    let over = hold
        .amount
        .checked_add(Amount::from_minor(1))
        .expect("fits");
    assert!(!settling.authorize_hold(
        &actor,
        hold,
        &mush_economy::HoldOp::Capture {
            hold: hold.id,
            amount: over,
        }
    ));
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
        assert!(
            seen.contains("escrow") || seen.contains("No such hold."),
            "wrong refusal: {seen}"
        );
        err(
            &mut tw,
            who,
            &format!("@hold/capture {hold}={} 100", r.a_acct),
        );
    }
    assert_eq!(
        held(&tw, r.j_acct),
        300,
        "a by-hand release freed a contract's escrow"
    );
    assert_eq!(
        bal(&tw, r.a_acct),
        0,
        "a by-hand capture spent a contract's escrow"
    );
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
    ok(
        tw,
        GOD,
        &format!("@asset/title #{}=#{}", widget.0, seller.0),
    );

    ok(tw, buyer, "@contract/draft Sale");
    let id = only_contract(tw);
    ok(tw, buyer, &format!("@contract/party {id}=#{}", seller.0));
    ok(tw, buyer, &format!("@contract/pay {id}={buyer_acct} 200"));
    ok(
        tw,
        buyer,
        &format!("@contract/split {id}={seller_acct} 1000‰"),
    );
    ok(
        tw,
        buyer,
        &format!("@contract/remainder {id}={seller_acct}"),
    );
    ok(
        tw,
        buyer,
        &format!(
            "@contract/deliver {id}=title #{} #{}->#{}",
            widget.0, seller.0, buyer.0
        ),
    );
    // The condition is the buyer confirming delivery; the seller's title is not a
    // condition but a **deliverable**, so the live check that matters at settlement
    // is D-E3-12's authority one: `ContractAuth` may only move a title whose current
    // owner is the party the surface named.
    ok(
        tw,
        buyer,
        &format!("@contract/condition {id}=attest party:#{}", buyer.0),
    );
    ok(tw, buyer, &format!("@contract/offer {id}"));
    ok(tw, buyer, &format!("@contract/accept {id}"));
    ok(tw, seller, &format!("@contract/accept {id}"));
    Sale {
        buyer,
        seller,
        buyer_acct,
        seller_acct,
        widget,
        id,
    }
}

#[test]
fn a_sale_moves_money_and_title_in_one_bundle_with_both_sides_accepted() {
    let mut tw = boot();
    let s = sale(&mut tw);
    assert_eq!(state(&tw, s.id), ContractState::Active);
    ok(&mut tw, s.buyer, &format!("@contract/attest {}=0", s.id));
    assert!(
        matches!(state(&tw, s.id), ContractState::Settled { .. }),
        "the sale did not settle"
    );
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
    ok(
        &mut tw,
        GOD,
        &format!("@asset/transfer #{}=#{}", s.widget.0, third.0),
    );
    ok(&mut tw, s.buyer, &format!("@contract/attest {}=0", s.id));
    assert_eq!(
        state(&tw, s.id),
        ContractState::ReadyToSettle,
        "a refused settlement moved on"
    );

    // Every fire re-verifies, refuses, and moves nothing; the attempts are bounded.
    for k in 0..(u32::from(mush_economy::MAX_SETTLE_ATTEMPTS) + 1) {
        sweep_at(&mut tw, 100 + i64::from(k) * 60);
    }
    assert_eq!(
        bal(&tw, s.buyer_acct),
        500,
        "a failed re-verification moved money"
    );
    assert_eq!(
        bal(&tw, s.seller_acct),
        0,
        "a failed re-verification paid the seller"
    );
    assert_eq!(
        held(&tw, s.buyer_acct),
        200,
        "the escrow was released by a failure"
    );
    assert_eq!(
        state(&tw, s.id),
        ContractState::Disputed,
        "the attempts were not bounded"
    );
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
    ok(
        tw,
        GOD,
        &format!(
            "@vault/create escrowbox=container:#{} owner=#{}",
            chest.0, seller.0
        ),
    );
    let vault = snap(tw).vaults().next().expect("vault").id;
    let widget = thing(tw, "widget");
    tw.world_mut()
        .move_to(widget, seller)
        .expect("hand it to the seller");
    ok(tw, seller, &format!("@vault/deposit #{}={vault}", widget.0));

    // The SELLER drafts: the escrow vault is theirs, and A2 checks vault viewability
    // at draft, so the party who can see the vault is the one who can name it. The
    // buyer adds their own consideration (A1's self-consideration rule).
    ok(tw, seller, "@contract/draft Delivery");
    let id = only_contract(tw);
    ok(tw, seller, &format!("@contract/party {id}=#{}", buyer.0));
    ok(tw, buyer, &format!("@contract/pay {id}={buyer_acct} 200"));
    ok(
        tw,
        seller,
        &format!("@contract/split {id}={seller_acct} 1000‰"),
    );
    ok(
        tw,
        seller,
        &format!("@contract/remainder {id}={seller_acct}"),
    );
    ok(
        tw,
        seller,
        &format!("@contract/condition {id}=deposit #{} {vault}", widget.0),
    );
    ok(tw, seller, &format!("@contract/offer {id}"));
    ok(tw, buyer, &format!("@contract/accept {id}"));
    ok(tw, seller, &format!("@contract/accept {id}"));
    Deposit {
        seller,
        buyer,
        buyer_acct,
        seller_acct,
        widget,
        vault,
        id,
    }
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
        err(
            &mut tw,
            who,
            &format!("@vault/withdraw #{}={}", d.widget.0, d.vault),
        );
    }
    let chest = snap(&tw)
        .vault(d.vault)
        .expect("vault")
        .backing
        .object()
        .expect("backing");
    assert_eq!(loc(&tw, d.widget), Some(chest), "the locked object moved");
    // The depositor the journal named is on the record.
    let c = contract(&tw, d.id);
    assert_eq!(c.deposits.len(), 1);
    assert_eq!(c.deposits[0].deposited_by, PrincipalId::Object(d.seller));

    // Staff cancel it: the state change lifts the lock, and the refund in the same
    // step releases the escrow AND hands the object back to its depositor.
    ok(&mut tw, GOD, &format!("@contract/cancel {}", d.id));
    assert_eq!(state(&tw, d.id), ContractState::Cancelled);
    assert_eq!(
        snap(&tw).custody_lock(d.widget, d.vault),
        None,
        "the lock survived the state change"
    );
    assert_eq!(
        held(&tw, d.buyer_acct),
        0,
        "the refund did not release the escrow"
    );
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
    tw.world_mut()
        .move_to(d.widget, d.seller)
        .expect("spirit it out");
    let before = journal_len(&tw);
    sweep_at(&mut tw, 100);
    assert_eq!(
        journal_len(&tw),
        before,
        "a failed re-verification posted something"
    );
    assert_eq!(
        bal(&tw, d.seller_acct),
        0,
        "a failed re-verification paid the seller"
    );
    assert_eq!(
        held(&tw, d.buyer_acct),
        200,
        "a failed re-verification released the escrow"
    );
    assert!(
        matches!(
            state(&tw, d.id),
            ContractState::Active | ContractState::ReadyToSettle
        ),
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
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/party {id}=#{}", r.a.0),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/pay {id}={} 100", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/split {id}={} 1000‰", r.a_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/remainder {id}={}", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/condition {id}=attest party:#{}", r.johnson.0),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/condition {id}=after 0 600"),
    );
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
    ok(&mut tw, r.a, &format!("@contract/accept {id}"));

    tw.set_now(1_000);
    ok(&mut tw, r.johnson, &format!("@contract/attest {id}=0"));
    assert_eq!(
        state(&tw, id),
        ContractState::Active,
        "the delay did not hold settlement back"
    );
    assert_eq!(bal(&tw, r.a_acct), 0);

    sweep_at(&mut tw, 1_100);
    assert_eq!(
        state(&tw, id),
        ContractState::Active,
        "the delay expired early"
    );
    sweep_at(&mut tw, 1_700);
    assert!(
        matches!(state(&tw, id), ContractState::Settled { .. }),
        "the sweep did not settle it"
    );
    assert_eq!(bal(&tw, r.a_acct), 100);
}

#[test]
fn an_expired_contract_refunds_and_a_forfeiting_one_pays_the_named_account() {
    for forfeit in [false, true] {
        let mut tw = boot();
        let r = run_rig(&mut tw, 3_000);
        ok(&mut tw, r.johnson, "@contract/draft Deadline");
        let id = only_contract(&tw);
        ok(
            &mut tw,
            r.johnson,
            &format!("@contract/party {id}=#{}", r.a.0),
        );
        ok(
            &mut tw,
            r.johnson,
            &format!("@contract/pay {id}={} 250", r.j_acct),
        );
        ok(
            &mut tw,
            r.johnson,
            &format!("@contract/split {id}={} 1000‰", r.a_acct),
        );
        ok(
            &mut tw,
            r.johnson,
            &format!("@contract/remainder {id}={}", r.j_acct),
        );
        // A condition nobody will satisfy, so the deadline is what happens.
        ok(
            &mut tw,
            r.johnson,
            &format!("@contract/condition {id}=attest party:#{}", r.a.0),
        );
        if forfeit {
            ok(
                &mut tw,
                r.johnson,
                &format!("@contract/deadline {id}=600 forfeit={}", r.b_acct),
            );
        } else {
            ok(
                &mut tw,
                r.johnson,
                &format!("@contract/deadline {id}=600 refund"),
            );
        }
        ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
        ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
        ok(&mut tw, r.a, &format!("@contract/accept {id}"));
        assert_eq!(held(&tw, r.j_acct), 250);

        sweep_at(&mut tw, 1_000);
        assert_eq!(
            state(&tw, id),
            ContractState::Expired,
            "the deadline did not lapse it"
        );
        assert_eq!(
            held(&tw, r.j_acct),
            0,
            "the escrow is still reserved after expiry"
        );
        if forfeit {
            assert_eq!(
                bal(&tw, r.b_acct),
                250,
                "the forfeit did not pay the named account"
            );
            assert_eq!(bal(&tw, r.j_acct), 3_000 - 250);
        } else {
            assert_eq!(bal(&tw, r.b_acct), 0, "a refund paid somebody");
            assert_eq!(
                bal(&tw, r.j_acct),
                3_000,
                "the refund did not give the money back"
            );
        }
    }
}

#[test]
fn an_offered_contract_that_lapses_refunds_even_when_its_terms_say_forfeit() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    ok(&mut tw, r.johnson, "@contract/draft Unbound");
    let id = only_contract(&tw);
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/party {id}=#{}", r.a.0),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/pay {id}={} 250", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/split {id}={} 1000‰", r.a_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/remainder {id}={}", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/deadline {id}=600 forfeit={}", r.b_acct),
    );
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    // Only the Johnson accepts: the agreement never came into force.
    ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));

    sweep_at(&mut tw, 1_000);
    assert_eq!(state(&tw, id), ContractState::Expired);
    assert_eq!(
        bal(&tw, r.b_acct),
        0,
        "an unbound contract forfeited somebody's money"
    );
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
    assert_eq!(
        state(&tw, id),
        ContractState::Disputed,
        "an attestation settled a disputed contract"
    );
    sweep_at(&mut tw, 100);
    assert_eq!(
        state(&tw, id),
        ContractState::Disputed,
        "a disputed contract settled"
    );
    assert_eq!(
        held(&tw, r.j_acct),
        300,
        "a disputed contract's escrow moved"
    );
    assert_eq!(bal(&tw, r.a_acct), 0);

    // Level 0: unanimity resolves it with no staff at all.
    for who in [r.johnson, r.a, r.b] {
        ok(&mut tw, who, &format!("@contract/agree {id}=refund"));
        assert_eq!(
            state(&tw, id),
            ContractState::Disputed,
            "a partial agreement resolved it"
        );
    }
    ok(&mut tw, r.c, &format!("@contract/agree {id}=refund"));
    assert_eq!(
        state(&tw, id),
        ContractState::Cancelled,
        "unanimity did not resolve the dispute"
    );
    assert_eq!(held(&tw, r.j_acct), 0);
    assert_eq!(bal(&tw, r.j_acct), 3_000);
}

/// Rewritten by the E4c verify fold (E4b). `@contract/resolve <id>=settle` now
/// records a **staff attestation** on every unmet `Attestation` condition, which is
/// the level-2 answer to evidence nobody can produce any more — a canon edge missed
/// under a freeze, an attestor who left the game. What it emphatically does *not* do
/// is decide a question of fact: a deterministic condition is still re-verified
/// against the live world, and still refuses (D-E4-4).
#[test]
fn staff_resolve_settles_a_dispute_and_still_re_verifies() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    let flat = thing(&mut tw, "flat");
    ok(&mut tw, r.johnson, "@contract/draft Extraction");
    let id = only_contract(&tw);
    for who in [r.a, r.b, r.c] {
        ok(
            &mut tw,
            r.johnson,
            &format!("@contract/party {id}=#{}", who.0),
        );
    }
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/pay {id}={} 300", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!(
            "@contract/split {id}={} 333‰ {} 333‰ {} 333‰",
            r.a_acct, r.b_acct, r.c_acct
        ),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/remainder {id}={}", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/condition {id}=attest party:#{}", r.johnson.0),
    );
    // The deterministic half: nobody holds title to the flat at all.
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/condition {id}=title #{} #{}", flat.0, r.a.0),
    );
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    accept_all(&mut tw, &r, id);
    ok(&mut tw, r.b, &format!("@contract/dispute {id}"));

    // Staff resolve attests the evidence condition — and settles nothing, because
    // the title condition is a fact about the world and is false.
    ok(
        &mut tw,
        GOD,
        &format!("@contract/resolve {id}=settle the fixer vanished"),
    );
    assert!(
        !matches!(state(&tw, id), ContractState::Settled { .. }),
        "a deterministic condition was overridden"
    );
    assert_ne!(
        bal(&tw, r.a_acct),
        99,
        "an unmet deterministic condition was paid out"
    );
    assert_eq!(
        held(&tw, r.j_acct),
        300,
        "a refused resolution released the escrow"
    );
    // The staff attestation stands on the document, with the reason, forever.
    let show = ok(&mut tw, GOD, &format!("@contract/show {id}"));
    assert!(
        show.contains("the fixer vanished"),
        "the resolve reason is not on the record: {show}"
    );

    // Make the world true: the ordinary retry path settles it, on the evidence staff
    // recorded and the fact the world now carries.
    ok(
        &mut tw,
        GOD,
        &format!("@asset/title #{}=#{}", flat.0, r.a.0),
    );
    sweep_at(&mut tw, 100);
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
    assert_eq!(
        after,
        vec![alert],
        "the ladder minted a second record for one subject"
    );

    // Acknowledge it without fixing the cause; past the cool-off it REOPENS in place.
    ok(&mut tw, GOD, &format!("@econ/ack {alert}"));
    assert!(!store(&tw).escalation(alert).expect("alert").is_open());
    sweep_at(&mut tw, 1_000 + 4 * 86_400 + 2 * 3_600);
    let record = store(&tw).escalation(alert).expect("alert");
    assert!(
        record.is_open(),
        "the acknowledged alert did not reopen when its cause persisted"
    );
    assert_eq!(record.reopened, 1, "the reopen tally was not kept");
    assert_eq!(
        store(&tw)
            .escalations()
            .filter(|e| e.subject == EscalationSubject::Contract(id))
            .count(),
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
    ok(
        tw,
        r.johnson,
        &format!("@contract/pay {id}={} 300", r.j_acct),
    );
    // Two mutually exclusive outcome conditions; nothing else gates the contract.
    ok(
        tw,
        r.johnson,
        &format!("@contract/condition {id}=attest party:#{}", r.a.0),
    );
    ok(
        tw,
        r.johnson,
        &format!("@contract/condition {id}=attest party:#{}", r.a.0),
    );
    ok(
        tw,
        r.johnson,
        &format!("@contract/adjudicator {id}=party:#{}", judge.0),
    );
    ok(
        tw,
        r.johnson,
        &format!("@contract/outcome {id}=success when=0 pay={}:300", r.a_acct),
    );
    ok(
        tw,
        r.johnson,
        &format!(
            "@contract/outcome {id}=failure when=1 pay={}:100,{}:200",
            r.a_acct, r.j_acct
        ),
    );
    // The single split is unused with a table, but a remainder account is still part
    // of a well-formed surface.
    ok(
        tw,
        r.johnson,
        &format!("@contract/remainder {id}={}", r.j_acct),
    );
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
    assert!(
        matches!(state(&tw, id), ContractState::Settled { .. }),
        "the outcome did not settle"
    );
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
    assert_eq!(
        state(&tw, id),
        ContractState::Active,
        "somebody else's attestation settled it"
    );
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
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/party {id}=#{}", r.a.0),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/pay {id}={} 300", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/condition {id}=attest party:#{}", judge.0),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/outcome {id}=paid when=0 pay={}:300", r.a_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/outcome {id}=kept when=0 pay={}:300", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/remainder {id}={}", r.j_acct),
    );
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
    ok(&mut tw, r.a, &format!("@contract/accept {id}"));

    let before = journal_len(&tw);
    ok(&mut tw, judge, &format!("@contract/attest {id}=0"));
    // §3.5: ambiguity is a dispute, at once — no retry can make a contract that does
    // not say what happened say it, so the attempts are not spent waiting.
    assert_eq!(
        state(&tw, id),
        ContractState::Disputed,
        "an ambiguous outcome did not dispute"
    );
    assert_eq!(
        journal_len(&tw),
        before,
        "an ambiguous outcome posted something"
    );
    assert_eq!(bal(&tw, r.a_acct), 0, "an ambiguous outcome paid a payee");
    assert_eq!(
        bal(&tw, r.j_acct),
        3_000,
        "an ambiguous outcome moved money"
    );
    assert_eq!(
        held(&tw, r.j_acct),
        300,
        "an ambiguous outcome released the escrow"
    );
    assert!(
        store(&tw)
            .escalations()
            .any(|e| e.subject == EscalationSubject::Contract(id)),
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
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/party {id}=#{}", r.watcher.0),
    );
    assert_eq!(contract(&tw, id).consent.parties.len(), 5);

    // The first acceptance freezes it, and a late party is refused.
    ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
    assert!(contract(&tw, id).roster_locked);
    let extra = tw.spawn_player("Late");
    let seen = err(
        &mut tw,
        r.johnson,
        &format!("@contract/party {id}=#{}", extra.0),
    );
    assert!(seen.contains("locked"), "wrong refusal: {seen}");
    err(&mut tw, r.johnson, &format!("@contract/roster {id}=open"));
    assert_eq!(
        contract(&tw, id).consent.parties.len(),
        5,
        "a late party joined a locked roster"
    );
}

/// Re-pinned to consent by authorship (E6a step 1, DL-01 Option A′): an amendment
/// voids and refunds every acceptance **but the author's own**. The Johnson wrote
/// the terms and accepted them, so his acceptance and his escrow stand; Alder's
/// acceptance — which escrowed nothing — is voided, and the amended terms are
/// re-collected from everyone else.
#[test]
fn an_amendment_refunds_every_other_partys_acceptance_and_keeps_the_authors() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    let id = offered_run(&mut tw, &r, 300);
    ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
    ok(&mut tw, r.a, &format!("@contract/accept {id}"));
    assert_eq!(held(&tw, r.j_acct), 300);

    ok(&mut tw, r.johnson, &format!("@contract/amend {id}"));
    let c = contract(&tw, id);
    assert_eq!(
        c.state,
        ContractState::Draft,
        "an amendment did not re-open the surface"
    );
    assert_eq!(
        c.acceptances.len(),
        1,
        "an amendment did not keep exactly the author's acceptance"
    );
    assert_eq!(c.acceptances[0].party, PrincipalId::Object(r.johnson));
    assert!(!c.roster_locked, "an amendment kept the roster locked");
    assert_eq!(
        held(&tw, r.j_acct),
        300,
        "an amendment moved the author's escrow"
    );
    assert_eq!(bal(&tw, r.j_acct), 3_000, "an amendment moved money");

    // Amend the shares, re-offer, re-collect from the OTHERS, and settle on the NEW
    // terms — the Johnson never re-accepts.
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/split {id}={} 1000‰", r.a_acct),
    );
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    for who in [r.a, r.b, r.c] {
        ok(&mut tw, who, &format!("@contract/accept {id}"));
    }
    assert_eq!(state(&tw, id), ContractState::Active);
    ok(&mut tw, r.johnson, &format!("@contract/attest {id}=0"));
    assert_eq!(
        bal(&tw, r.a_acct),
        300,
        "the amended split did not take effect"
    );
    assert_eq!(bal(&tw, r.b_acct), 0);
    assert_eq!(held(&tw, r.j_acct), 0);
}

/// Re-pinned to A′: a refund posts only for a party **other than** the author, so
/// the generation moves on a fixer-issuer's amendment (the payer's escrow comes
/// back) and the next refund of the same id must not hit that spent key.
#[test]
fn an_amended_contract_refunds_again_instead_of_hitting_its_first_refunds_key() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    let id = fixer_issued_run(&mut tw, &r, 300);
    ok(&mut tw, r.a, &format!("@contract/accept {id}"));
    assert_eq!(held(&tw, r.a_acct), 300);
    ok(&mut tw, r.johnson, &format!("@contract/amend {id}"));
    assert_eq!(
        contract(&tw, id).amendments,
        1,
        "the amendment did not advance the key generation"
    );
    assert_eq!(held(&tw, r.a_acct), 0);

    // The SAME contract id, offered and funded again, and then cancelled. Its refund
    // must actually run: at generation 0 the key would resolve to the amendment's own
    // refund as an idempotency HIT and hand nothing back.
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    ok(&mut tw, r.a, &format!("@contract/accept {id}"));
    assert_eq!(held(&tw, r.a_acct), 300);
    ok(&mut tw, r.johnson, &format!("@contract/cancel {id}"));
    assert_eq!(
        held(&tw, r.a_acct),
        0,
        "the second refund hit the first refund's key"
    );
    assert_eq!(bal(&tw, r.a_acct), 500);

    // Generation 0 is byte-identical to what E4a specified, so every stored record
    // keeps resolving; only the amended generations move.
    assert_eq!(
        mush_economy::contract_op_key(id, "refund", 0),
        mush_economy::refund_key(id)
    );
    assert_ne!(
        mush_economy::contract_op_key(id, "refund", 1),
        mush_economy::refund_key(id)
    );
}

#[test]
fn the_five_roles_are_recorded_separately_and_none_of_them_is_authority() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    let author = tw.spawn_player("Author");
    let judge = r.watcher;
    ok(&mut tw, r.johnson, "@contract/draft Roles");
    let id = only_contract(&tw);
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/party {id}=#{}", r.a.0),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/pay {id}={} 100", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/split {id}={} 1000‰", r.a_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/remainder {id}={}", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/condition {id}=attest party:#{}", r.a.0),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/adjudicator {id}=party:#{}", judge.0),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/role {id}=author #{}", author.0),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/role {id}=johnson #{}", r.johnson.0),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/role {id}=facilitator #{}", judge.0),
    );
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));

    let c = contract(&tw, id);
    assert_eq!(c.roles.author, Some(PrincipalId::Object(author)));
    assert_eq!(
        c.roles.payer,
        Some(PrincipalId::Object(r.johnson)),
        "the payer is the account's owner"
    );
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
    assert_eq!(
        bal(&tw, r.j_acct),
        3_000 - 100,
        "the payer's account is what funded it"
    );
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
        assert!(
            seen.contains("No such contract."),
            "`{line}` leaked: {seen}"
        );
    }
    // ...and it is not discoverable through the list either.
    let seen = ok(&mut tw, stranger, "@contract/list");
    assert!(
        seen.contains("No contracts."),
        "the list leaked a contract: {seen}"
    );
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
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/party {id}=#{}", r.a.0),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/pay {id}={} 100", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/split {id}={} 1000‰", r.a_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/remainder {id}={}", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/visibility {id}=audience #{}", r.watcher.0),
    );
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));

    // T6: visible to the eligible defenders, without being published.
    let seen = ok(&mut tw, r.watcher, &format!("@contract/show {id}"));
    assert!(
        seen.contains("Operation"),
        "the audience could not read it: {seen}"
    );
    assert!(
        seen.contains("audience"),
        "the visibility class is not shown: {seen}"
    );
    let seen = err(&mut tw, stranger, &format!("@contract/show {id}"));
    assert!(
        seen.contains("No such contract."),
        "an audience contract leaked: {seen}"
    );
    // An audience contract is emphatically NOT public.
    assert!(!contract(&tw, id).consent.visibility.is_public());

    // The audience is canonicalized by principal key inside the consent hash, so
    // re-typing it in another order is the SAME surface.
    let hash = contract(&tw, id).consent_hash;
    let mut c = contract(&tw, id);
    c.consent.visibility = mush_economy::EconomicVisibility::Audience {
        members: vec![PrincipalId::Object(r.watcher)],
        announce_issuer: false,
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
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/party {id}=#{}", r.a.0),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/pay {id}={} 100", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/split {id}={} 1000‰", r.a_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/remainder {id}={}", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/visibility {id}=public"),
    );
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));

    // Anyone may read it...
    ok(&mut tw, stranger, &format!("@contract/show {id}"));
    // ...and every party's acceptance binds to a surface that SAYS so: flipping the
    // visibility after Offer is refused, exactly like any other mutation.
    ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
    err(
        &mut tw,
        r.johnson,
        &format!("@contract/visibility {id}=parties"),
    );
    let c = contract(&tw, id);
    assert!(c.consent.visibility.is_public());
    assert!(c.accepted_by(&PrincipalId::Object(r.johnson)));
    // A party reads the other parties as the surface names them, and no balances.
    let seen = ok(&mut tw, r.a, &format!("@contract/show {id}"));
    assert!(seen.contains("parties:"), "{seen}");
    assert!(
        !seen.contains("posted"),
        "a contract view leaked a balance: {seen}"
    );
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
    ok(
        &mut tw,
        GOD,
        &format!("@asset/title #{}=#{}", flat.0, landlord.0),
    );

    ok(&mut tw, landlord, "@contract/draft Lease");
    let id = only_contract(&tw);
    ok(
        &mut tw,
        landlord,
        &format!("@contract/party {id}=#{}", tenant.0),
    );
    // A1: the landlord cannot name the tenant's wallet; the tenant adds it.
    ok(&mut tw, tenant, &format!("@contract/pay {id}={t_acct} 100"));
    ok(
        &mut tw,
        landlord,
        &format!("@contract/split {id}={l_acct} 1000‰"),
    );
    ok(
        &mut tw,
        landlord,
        &format!("@contract/remainder {id}={l_acct}"),
    );
    ok(
        &mut tw,
        landlord,
        &format!(
            "@contract/deliver {id}=collateral #{} #{}",
            flat.0, landlord.0
        ),
    );
    ok(
        &mut tw,
        landlord,
        &format!("@contract/deliver {id}=obligation {t_acct} {l_acct} 50 every=600"),
    );
    ok(
        &mut tw,
        landlord,
        &format!("@contract/condition {id}=attest party:#{}", landlord.0),
    );
    ok(&mut tw, landlord, &format!("@contract/offer {id}"));
    ok(&mut tw, landlord, &format!("@contract/accept {id}"));
    ok(&mut tw, tenant, &format!("@contract/accept {id}"));
    assert_eq!(state(&tw, id), ContractState::Active);

    // A contract that also creates an obligation pledges its collateral as a LEASE.
    let asset = snap(&tw).asset(flat).expect("asset");
    assert_eq!(
        asset.encumbrances,
        vec![Encumbrance::Lease {
            lessor: PrincipalId::Object(landlord),
            contract: id
        }],
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
        snap(&tw)
            .asset(flat)
            .expect("asset")
            .encumbrances
            .is_empty(),
        "the lease claim stranded on the asset"
    );
    assert_eq!(
        held(&tw, t_acct),
        0,
        "the refund did not release the escrow"
    );
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
    ok(
        &mut tw,
        GOD,
        &format!("@asset/title #{}=#{}", flat.0, landlord.0),
    );

    ok(&mut tw, landlord, "@contract/draft Pledge");
    let id = only_contract(&tw);
    ok(
        &mut tw,
        landlord,
        &format!("@contract/party {id}=#{}", tenant.0),
    );
    ok(&mut tw, tenant, &format!("@contract/pay {id}={t_acct} 100"));
    ok(
        &mut tw,
        landlord,
        &format!("@contract/split {id}={l_acct} 1000‰"),
    );
    ok(
        &mut tw,
        landlord,
        &format!("@contract/remainder {id}={l_acct}"),
    );
    ok(
        &mut tw,
        landlord,
        &format!(
            "@contract/deliver {id}=collateral #{} #{}",
            flat.0, landlord.0
        ),
    );
    ok(&mut tw, landlord, &format!("@contract/offer {id}"));
    // Only the tenant accepts, so the claim is never placed — activation is what
    // places it. Cancelling now must still hand the escrow back: a `Release` of an
    // encumbrance the asset does not carry would fail the whole refund plan.
    ok(&mut tw, tenant, &format!("@contract/accept {id}"));
    assert_eq!(held(&tw, t_acct), 100);
    assert!(
        snap(&tw)
            .asset(flat)
            .expect("asset")
            .encumbrances
            .is_empty()
    );

    ok(&mut tw, landlord, &format!("@contract/cancel {id}"));
    assert_eq!(state(&tw, id), ContractState::Cancelled);
    assert_eq!(
        held(&tw, t_acct),
        0,
        "the refund stranded the escrow on an unplaced pledge"
    );
    assert_eq!(bal(&tw, t_acct), 1_000);
}

// ======================================================================
// Boot: reconciliation at every state
// ======================================================================

#[test]
fn every_contract_state_survives_a_reboot() {
    // Draft / Offered / Active / Disputed / Settled / Cancelled, each driven to that
    // state through the real commands and then replayed through the real boot.
    for target in [
        "draft",
        "offered",
        "active",
        "disputed",
        "settled",
        "cancelled",
    ] {
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
        assert!(
            !store(&tw).is_frozen(),
            "{target}: a clean reboot froze the economy"
        );
        assert_eq!(
            state(&tw, id),
            before,
            "{target}: the state did not survive the reboot"
        );
        assert_eq!(
            held(&tw, r.j_acct),
            held_before,
            "{target}: the escrow changed across a reboot"
        );
        assert_eq!(
            bal(&tw, r.j_acct),
            bal_before,
            "{target}: a balance changed across a reboot"
        );
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
    assert_eq!(
        state(&tw, id),
        ContractState::Disputed,
        "boot did not quarantine a broken escrow"
    );
    assert!(
        !store(&tw).is_frozen(),
        "one broken contract froze the whole economy"
    );
    assert!(
        store(&tw)
            .escalations()
            .any(|e| e.subject == EscalationSubject::Contract(id)),
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
    assert_eq!(
        snap(&tw).account(r.j_acct).expect("account").status,
        AccountStatus::Frozen
    );
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
    assert_eq!(
        ids,
        vec![0, 1, 2],
        "the contract allocator re-minted a live id: {ids:?}"
    );
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
    assert_eq!(
        snap(&tw).contracts().count(),
        mush_economy::MAX_OPEN_CONTRACTS_PER_ISSUER
    );

    // Eight days later the sweep collects every abandoned draft, and the issuer can
    // open new ones again.
    sweep_at(&mut tw, 1_000 + 8 * 86_400);
    assert_eq!(
        snap(&tw).contracts().count(),
        0,
        "abandoned drafts were not collected"
    );
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
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/party {id}=#{}", r.a.0),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/pay {id}={} 100", r.j_acct),
    );
    // No remainder account at all: a floored split would have nowhere to land.
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/split {id}={} 500‰", r.a_acct),
    );
    err(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    assert_eq!(state(&tw, id), ContractState::Draft);

    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/remainder {id}={}", r.j_acct),
    );
    // A split that pays out more than is escrowed is infeasible at Offer.
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/split {id}={} 500", r.a_acct),
    );
    err(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/split {id}={} 60", r.a_acct),
    );
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    assert_eq!(state(&tw, id), ContractState::Offered);
}

#[test]
fn the_permitted_evidence_list_is_exhaustive_when_it_is_set() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    ok(&mut tw, r.johnson, "@contract/draft Evidence");
    let id = only_contract(&tw);
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/party {id}=#{}", r.a.0),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/pay {id}={} 100", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/split {id}={} 1000‰", r.a_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/remainder {id}={}", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/condition {id}=before 600"),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/evidence {id}=attest"),
    );
    // The surface says attestations only, and it carries a deadline condition.
    err(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/evidence {id}=attest,before"),
    );
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
}

// ======================================================================
// ---- E4b2 verify fold ----
//
// Lens A (command authority + existence hiding), lens M (consent and settlement
// money), lens B (durability, boot, the sweep's contract step). One test per
// numbered finding; every one asserts against balances, `held_amount`, object
// locations and the journal, not just against the message.
// ======================================================================

/// A1 — `@contract/pay` was an account existence-and-owner oracle.
#[test]
fn vf_a1_pay_names_only_a_viewable_account_and_a_party_adds_their_own() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    ok(&mut tw, r.johnson, "@contract/draft Oracle");
    let id = only_contract(&tw);
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/party {id}=#{}", r.a.0),
    );

    // (a) The issuer probing a live account they may not view gets the same text as
    // an account that does not exist at all.
    let hidden = err(
        &mut tw,
        r.johnson,
        &format!("@contract/pay {id}={} 100", r.a_acct),
    );
    let absent = err(&mut tw, r.johnson, &format!("@contract/pay {id}=4242 100"));
    assert_eq!(
        hidden, absent,
        "an unviewable account and a missing one read differently"
    );
    assert!(
        hidden.contains("No such account."),
        "wrong refusal: {hidden}"
    );
    assert!(
        contract(&tw, id).consent.terms.consideration.is_empty(),
        "a refused /pay still wrote a consideration"
    );

    // (b) The party adds it themselves, and the confirmation echoes no owner.
    let seen = ok(
        &mut tw,
        r.a,
        &format!("@contract/pay {id}={} 100", r.a_acct),
    );
    assert!(
        !seen.contains("Alder"),
        "the confirmation named the account's owner: {seen}"
    );
    let c = contract(&tw, id);
    assert_eq!(c.consent.terms.consideration.len(), 1);
    assert_eq!(
        c.consent.terms.consideration[0].from,
        PrincipalId::Object(r.a)
    );

    // A non-party cannot, and the contract does not exist as far as they are told.
    let w_acct = account_of(&tw, r.watcher);
    let seen = err(
        &mut tw,
        r.watcher,
        &format!("@contract/pay {id}={w_acct} 50"),
    );
    assert!(seen.contains("No such contract."), "wrong refusal: {seen}");

    // ...and `/show` never derives a name from an account the viewer cannot view.
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/pay {id}={} 100", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/split {id}={} 1000‰", r.a_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/remainder {id}={}", r.j_acct),
    );
    let seen = ok(&mut tw, r.johnson, &format!("@contract/show {id}"));
    assert!(
        seen.contains("an account"),
        "an unviewable account id was printed: {seen}"
    );
}

/// A2 — `contract_refusal` bypassed the existence-hiding renderer.
#[test]
fn vf_a2_a_contract_refusal_hides_a_resource_the_actor_may_not_view() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    // A vault the Johnson has no standing over at all.
    let chest = thing(&mut tw, "chest");
    ok(
        &mut tw,
        GOD,
        &format!(
            "@vault/create theirs=container:#{} owner=#{}",
            chest.0, r.a.0
        ),
    );
    let vault = snap(&tw).vaults().next().expect("vault").id;
    let widget = thing(&mut tw, "widget");

    ok(&mut tw, r.johnson, "@contract/draft Probe");
    let id = only_contract(&tw);
    // At DRAFT: an unviewable vault and a nonexistent one are one message.
    let hidden = err(
        &mut tw,
        r.johnson,
        &format!("@contract/condition {id}=deposit #{} {vault}", widget.0),
    );
    let absent = err(
        &mut tw,
        r.johnson,
        &format!("@contract/condition {id}=deposit #{} 999", widget.0),
    );
    assert_eq!(
        hidden, absent,
        "an unviewable vault and a missing one read differently at draft"
    );
    assert!(hidden.contains("No such vault."), "wrong refusal: {hidden}");

    // At OFFER: the same rule, through `contract_refusal`. A closed account the
    // Johnson may not view renders exactly like an account id that never existed.
    let w_acct = account_of(&tw, r.watcher);
    ok(&mut tw, GOD, &format!("@account/close {w_acct}"));
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/party {id}=#{}", r.a.0),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/pay {id}={} 100", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/split {id}={} 1000‰", r.a_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/remainder {id}={w_acct}"),
    );
    let hidden = err(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/remainder {id}=4242"),
    );
    let absent = err(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    assert_eq!(
        hidden, absent,
        "an unviewable account and a missing one read differently at offer"
    );
    assert!(
        hidden.contains("No such account."),
        "wrong refusal: {hidden}"
    );
    assert_eq!(state(&tw, id), ContractState::Draft);

    // Staff, who may view everything, still see the real reason.
    let staff = err(&mut tw, GOD, &format!("@contract/offer {id}"));
    assert!(
        staff.contains("no usable account"),
        "staff got the hidden text: {staff}"
    );
}

/// A3 — an org attestor could never attest, and an unsatisfiable set was offerable.
#[test]
fn vf_a3_an_org_attestor_can_attest_and_an_unsatisfiable_set_is_refused() {
    let mut tw = boot();
    let o = org_rig(&mut tw);

    // (1) A `Capability` set with no scope names a compartment that does not exist,
    // so nobody can ever hold it: the condition could never be discharged and the
    // escrow behind it would sit until the deadline with nothing saying why.
    ok(
        &mut tw,
        GOD,
        &format!("@contract/draft Impossible scope=#{}", o.scope.0),
    );
    let bad = only_contract(&tw);
    ok(
        &mut tw,
        GOD,
        &format!("@contract/party {bad}=#{}", o.payee.0),
    );
    ok(&mut tw, GOD, &format!("@contract/pay {bad}={} 400", o.acct));
    ok(
        &mut tw,
        GOD,
        &format!("@contract/split {bad}={} 1000‰", o.payee_acct),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@contract/remainder {bad}={}", o.acct),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@contract/condition {bad}=attest cap:contract.attest"),
    );
    let seen = err(&mut tw, GOD, &format!("@contract/offer {bad}"));
    assert!(seen.contains("nobody can satisfy"), "wrong refusal: {seen}");
    assert_eq!(state(&tw, bad), ContractState::Draft);
    ok(&mut tw, GOD, &format!("@contract/cancel {bad}"));

    // (2) `party:org:Ironhold` — an organization types nothing, so whoever may spend
    // its money attests for it, and the RECORD names the org rather than the officer
    // (so the attestation stands once given, as a party attestation must).
    ok(
        &mut tw,
        GOD,
        &format!("@contract/draft Supply scope=#{}", o.scope.0),
    );
    let id = snap(&tw)
        .contracts()
        .map(|c| c.id)
        .max()
        .expect("second contract");
    ok(
        &mut tw,
        GOD,
        &format!("@contract/party {id}=#{}", o.payee.0),
    );
    ok(&mut tw, GOD, &format!("@contract/pay {id}={} 400", o.acct));
    ok(
        &mut tw,
        GOD,
        &format!("@contract/split {id}={} 1000‰", o.payee_acct),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@contract/remainder {id}={}", o.acct),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@contract/condition {id}=attest party:org:Ironhold"),
    );
    ok(&mut tw, GOD, &format!("@contract/offer {id}"));
    ok(
        &mut tw,
        GOD,
        &format!(
            "@org/grant #{}=Ironhold contract.accept scope=#{}",
            o.member.0, o.scope.0
        ),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@org/grant #{}=Ironhold economy.account.post", o.member.0),
    );
    ok(&mut tw, o.member, &format!("@contract/accept {id}"));
    ok(&mut tw, o.payee, &format!("@contract/accept {id}"));
    assert_eq!(state(&tw, id), ContractState::Active);

    ok(&mut tw, o.member, &format!("@contract/attest {id}=0"));
    assert!(
        matches!(state(&tw, id), ContractState::Settled { .. }),
        "an org attestation did not settle it"
    );
    assert_eq!(bal(&tw, o.payee_acct), 400);
    // The record names the ORG, not Quinn: that is what makes it re-verify.
    let c = contract(&tw, id);
    assert!(
        matches!(&c.satisfied[0], Some(a) if a.by == mush_economy::AttestorRef::Principal(PrincipalId::Organization(o.org))),
        "the attestation named the officer instead of the party: {:?}",
        c.satisfied[0]
    );
}

/// A4 — an org party on an **unscoped** contract could never act.
#[test]
fn vf_a4_an_org_party_on_an_unscoped_contract_can_accept_dispute_and_agree() {
    let mut tw = boot();
    let o = org_rig(&mut tw);
    let issuer = tw.spawn_player("Fixer");
    ok(&mut tw, GOD, &format!("@account/open #{}=gold", issuer.0));
    let i_acct = account_of(&tw, issuer);
    let faucet = snap(&tw)
        .accounts()
        .find(|x| matches!(x.owner, PrincipalId::System(_)))
        .expect("faucet")
        .id;
    ok(&mut tw, GOD, &format!("@econ/mint {faucet}={i_acct} 500"));

    // No `scope=`, so there is no compartment and the capability route cannot apply.
    ok(&mut tw, issuer, "@contract/draft Unscoped");
    let id = only_contract(&tw);
    ok(
        &mut tw,
        issuer,
        &format!("@contract/party {id}=org:Ironhold"),
    );
    ok(&mut tw, issuer, &format!("@contract/pay {id}={i_acct} 100"));
    ok(
        &mut tw,
        issuer,
        &format!("@contract/split {id}={} 1000‰", o.payee_acct),
    );
    ok(
        &mut tw,
        issuer,
        &format!("@contract/remainder {id}={i_acct}"),
    );
    ok(
        &mut tw,
        issuer,
        &format!("@contract/condition {id}=attest party:#{}", issuer.0),
    );
    ok(&mut tw, issuer, &format!("@contract/offer {id}"));
    ok(&mut tw, issuer, &format!("@contract/accept {id}"));

    // Whoever may spend the org's money speaks for it — the same predicate a capture
    // and `may_view_hold` ask.
    err(&mut tw, o.member, &format!("@contract/accept {id}"));
    ok(
        &mut tw,
        GOD,
        &format!("@org/grant #{}=Ironhold economy.account.post", o.member.0),
    );
    ok(&mut tw, o.member, &format!("@contract/accept {id}"));
    assert_eq!(
        state(&tw, id),
        ContractState::Active,
        "the org's member could not accept for it"
    );
    ok(&mut tw, o.member, &format!("@contract/dispute {id}"));
    assert_eq!(state(&tw, id), ContractState::Disputed);
    ok(&mut tw, o.member, &format!("@contract/agree {id}=refund"));
    ok(&mut tw, issuer, &format!("@contract/agree {id}=refund"));
    assert_eq!(
        state(&tw, id),
        ContractState::Cancelled,
        "unanimity did not resolve it"
    );
    assert_eq!(
        held(&tw, i_acct),
        0,
        "the agreed refund did not release the escrow"
    );
    assert_eq!(bal(&tw, i_acct), 500);
}

/// A5 — `/agree` accepted `contract.dispute`, which is the freeze right, not the
/// disposition right.
#[test]
fn vf_a5_agreeing_needs_the_accept_right_and_disputing_does_not() {
    let mut tw = boot();
    let o = org_rig(&mut tw);
    ok(
        &mut tw,
        GOD,
        &format!("@contract/draft Disposition scope=#{}", o.scope.0),
    );
    let id = only_contract(&tw);
    ok(
        &mut tw,
        GOD,
        &format!("@contract/party {id}=#{}", o.payee.0),
    );
    ok(&mut tw, GOD, &format!("@contract/pay {id}={} 400", o.acct));
    ok(
        &mut tw,
        GOD,
        &format!("@contract/split {id}={} 1000‰", o.payee_acct),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@contract/remainder {id}={}", o.acct),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@contract/condition {id}=attest staff"),
    );
    ok(&mut tw, GOD, &format!("@contract/offer {id}"));
    ok(
        &mut tw,
        GOD,
        &format!(
            "@org/grant #{}=Ironhold contract.accept scope=#{}",
            o.member.0, o.scope.0
        ),
    );
    ok(
        &mut tw,
        GOD,
        &format!(
            "@org/grant #{}=Ironhold economy.account.post scope=#{}",
            o.member.0, o.scope.0
        ),
    );
    ok(&mut tw, o.member, &format!("@contract/accept {id}"));
    ok(&mut tw, o.payee, &format!("@contract/accept {id}"));
    assert_eq!(state(&tw, id), ContractState::Active);

    // A second member with the FREEZE right only.
    let watcher = tw.spawn_player("Watcher");
    ok(
        &mut tw,
        GOD,
        &format!(
            "@org/grant #{}=Ironhold contract.dispute scope=#{}",
            watcher.0, o.scope.0
        ),
    );
    ok(&mut tw, watcher, &format!("@contract/dispute {id}"));
    assert_eq!(state(&tw, id), ContractState::Disputed);
    let seen = err(&mut tw, watcher, &format!("@contract/agree {id}=settle"));
    assert!(seen.contains("No such contract."), "wrong refusal: {seen}");
    assert!(
        contract(&tw, id).agreements.is_empty(),
        "a dispute-only member recorded an agreement"
    );

    // The accept-level right disposes of it.
    ok(&mut tw, o.member, &format!("@contract/agree {id}=refund"));
    assert_eq!(contract(&tw, id).agreements.len(), 1);
}

/// A6 — a settlement refusal reached a party raw.
#[test]
fn vf_a6_a_settlement_refusal_does_not_leak_another_partys_account_status() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    ok(&mut tw, r.johnson, "@contract/draft Leak");
    let id = only_contract(&tw);
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/party {id}=#{}", r.a.0),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/pay {id}={} 100", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/split {id}={} 1000‰", r.a_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/remainder {id}={}", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/condition {id}=attest party:#{}", r.johnson.0),
    );
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
    ok(&mut tw, r.a, &format!("@contract/accept {id}"));
    ok(&mut tw, r.a, &format!("@contract/dispute {id}"));
    // Alder closes her own wallet; the settlement can no longer pay her.
    ok(&mut tw, r.a, &format!("@account/close {}", r.a_acct));

    ok(&mut tw, r.a, &format!("@contract/agree {id}=settle"));
    let seen = ok(&mut tw, r.johnson, &format!("@contract/agree {id}=settle"));
    assert!(
        seen.contains("staff have been paged"),
        "the Johnson was told about an account he may not view: {seen}"
    );
    assert!(
        !seen.contains("no usable account"),
        "the raw refusal reached a party: {seen}"
    );
    assert_eq!(bal(&tw, r.a_acct), 0, "a refused settlement paid somebody");
    assert_eq!(
        held(&tw, r.j_acct),
        100,
        "a refused settlement released the escrow"
    );
}

/// A7 — an `Audience` was rendered exactly like a party.
#[test]
fn vf_a7_an_audience_gets_a_notice_and_a_public_stranger_gets_no_hidden_ids() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    ok(&mut tw, r.johnson, "@contract/draft Operation");
    let id = only_contract(&tw);
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/party {id}=#{}", r.a.0),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/pay {id}={} 300", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/split {id}={} 1000‰", r.a_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/remainder {id}={}", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/condition {id}=attest party:#{}", r.johnson.0),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/deadline {id}=600 refund"),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/visibility {id}=audience #{}", r.watcher.0),
    );
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
    ok(&mut tw, r.a, &format!("@contract/accept {id}"));

    // The audience is told there IS an operation, and when — nothing else.
    let seen = ok(&mut tw, r.watcher, &format!("@contract/show {id}"));
    assert!(
        seen.contains("Operation"),
        "the audience could not read the notice: {seen}"
    );
    assert!(
        seen.contains("window closes"),
        "the notice does not carry the window: {seen}"
    );
    assert!(
        seen.contains("Attestation"),
        "the notice does not carry the condition kinds: {seen}"
    );
    assert!(!seen.contains("300"), "the notice leaked an amount: {seen}");
    assert!(!seen.contains("Alder"), "the notice leaked a party: {seen}");
    assert!(
        !seen.contains("escrow holds"),
        "the notice leaked a hold id: {seen}"
    );
    assert!(
        !seen.contains("issued by"),
        "the notice named the issuer by default: {seen}"
    );
    // A party still sees the whole surface.
    let full = ok(&mut tw, r.a, &format!("@contract/show {id}"));
    assert!(
        full.contains("Alder"),
        "a party lost the party list: {full}"
    );
    assert!(
        full.contains("escrow holds"),
        "a party lost the escrow ids: {full}"
    );

    // A public contract: a stranger reads the surface as consented, with every
    // account id they may not view masked.
    let stranger = tw.spawn_player("Nobody");
    ok(&mut tw, r.johnson, "@contract/draft Open");
    let pid = snap(&tw)
        .contracts()
        .map(|c| c.id)
        .max()
        .expect("second contract");
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/party {pid}=#{}", r.a.0),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/pay {pid}={} 100", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/split {pid}={} 1000‰", r.a_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/remainder {pid}={}", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/visibility {pid}=public"),
    );
    ok(&mut tw, r.johnson, &format!("@contract/offer {pid}"));
    let seen = ok(&mut tw, stranger, &format!("@contract/show {pid}"));
    assert!(
        seen.contains("Johnson"),
        "the consented surface lost its parties: {seen}"
    );
    assert!(
        seen.contains("an account"),
        "a public stranger read an account id: {seen}"
    );
    assert!(
        !seen.contains(&format!("account #{}", r.j_acct)),
        "a public stranger read an account id they may not view: {seen}"
    );
}

/// A8 — the help text and the parser disagreed about `condition … title`.
#[test]
fn vf_a8_help_matches_the_parser_for_title_and_names_who_may_run_what() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    let seen = ok(&mut tw, r.johnson, "help @contract");
    assert!(
        seen.contains("title <obj> <principal>"),
        "help still documents an attestor: {seen}"
    );
    assert!(
        seen.contains("no party: prefix"),
        "help does not say the prefix is wrong: {seen}"
    );
    assert!(
        seen.contains("ISSUER or staff"),
        "help does not say who may offer: {seen}"
    );
    assert!(
        seen.contains("a PARTY"),
        "help does not say who may accept: {seen}"
    );

    // …and the parser agrees.
    let widget = thing(&mut tw, "widget");
    ok(
        &mut tw,
        GOD,
        &format!("@asset/title #{}=#{}", widget.0, r.a.0),
    );
    ok(&mut tw, r.johnson, "@contract/draft Help");
    let id = only_contract(&tw);
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/condition {id}=title #{} #{}", widget.0, r.a.0),
    );
    let seen = err(
        &mut tw,
        r.johnson,
        &format!(
            "@contract/condition {id}=title #{} party:#{}",
            widget.0, r.a.0
        ),
    );
    assert!(
        seen.contains("No such principal."),
        "the parser took a party: prefix: {seen}"
    );
}

// ---- lens M: consent and settlement money ----------------------------------

/// M1 — a contract could encumber a bystander's asset.
#[test]
fn vf_m1_a_bystanders_asset_is_never_encumbered() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    let bystander = tw.spawn_player("Bystander");
    let heirloom = thing(&mut tw, "heirloom");
    ok(
        &mut tw,
        GOD,
        &format!("@asset/title #{}=#{}", heirloom.0, bystander.0),
    );

    ok(&mut tw, r.johnson, "@contract/draft Pledge");
    let id = only_contract(&tw);
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/party {id}=#{}", r.a.0),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/pay {id}={} 100", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/split {id}={} 1000‰", r.a_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/remainder {id}={}", r.j_acct),
    );
    // Somebody who is not a party, pledging something that is not the parties' to
    // pledge. A pledge encumbers at activation, and an encumbered asset cannot be
    // transferred — so this is a claim placed on a stranger's property by an
    // agreement they were never shown.
    ok(
        &mut tw,
        r.johnson,
        &format!(
            "@contract/deliver {id}=collateral #{} #{}",
            heirloom.0, bystander.0
        ),
    );
    let seen = err(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    assert!(seen.contains("not a party"), "wrong refusal: {seen}");
    assert_eq!(state(&tw, id), ContractState::Draft);
    assert!(
        snap(&tw)
            .asset(heirloom)
            .expect("asset")
            .encumbrances
            .is_empty(),
        "a refused offer still encumbered a bystander's asset"
    );

    // Naming them a party is not enough either: the pledger must hold the title.
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/party {id}=#{}", bystander.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@asset/transfer #{}=#{}", heirloom.0, r.a.0),
    );
    let seen = err(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    assert!(seen.contains("No such asset."), "wrong refusal: {seen}");
    assert_eq!(state(&tw, id), ContractState::Draft);

    // With the title where the surface says it is, the offer stands and the claim is
    // placed at activation — and comes off again whatever happens.
    ok(
        &mut tw,
        GOD,
        &format!("@asset/transfer #{}=#{}", heirloom.0, bystander.0),
    );
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
    ok(&mut tw, r.a, &format!("@contract/accept {id}"));
    ok(&mut tw, bystander, &format!("@contract/accept {id}"));
    assert_eq!(state(&tw, id), ContractState::Active);
    assert_eq!(
        snap(&tw).asset(heirloom).expect("asset").encumbrances.len(),
        1,
        "the consented pledge was not placed"
    );
    ok(&mut tw, GOD, &format!("@contract/cancel {id}"));
    assert!(
        snap(&tw)
            .asset(heirloom)
            .expect("asset")
            .encumbrances
            .is_empty(),
        "the pledge did not come back"
    );
}

/// M2 + M3 + B3 — a refused forfeit was silent and terminal, and the escrow was
/// stranded on a contract nothing could reach.
#[test]
fn vf_m2_a_refused_forfeit_leaves_a_live_contract_and_recoverable_escrow() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    ok(&mut tw, r.johnson, "@contract/draft Forfeit");
    let id = only_contract(&tw);
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/party {id}=#{}", r.a.0),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/pay {id}={} 250", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/split {id}={} 1000‰", r.a_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/remainder {id}={}", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/condition {id}=attest party:#{}", r.a.0),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/deadline {id}=600 forfeit={}", r.b_acct),
    );
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
    ok(&mut tw, r.a, &format!("@contract/accept {id}"));
    // The forfeit target closes before the deadline.
    ok(&mut tw, r.b, &format!("@account/close {}", r.b_acct));

    sweep_at(&mut tw, 1_000);
    // **Terminal only after the post.** The contract is still live, the escrow is
    // still reserved and reachable, and a human has been told.
    assert!(
        matches!(
            state(&tw, id),
            ContractState::Active | ContractState::ReadyToSettle
        ),
        "a refused forfeit made the contract terminal: {:?}",
        state(&tw, id)
    );
    assert_eq!(
        held(&tw, r.j_acct),
        250,
        "a refused forfeit lost the escrow"
    );
    assert_eq!(
        bal(&tw, r.b_acct),
        0,
        "a refused forfeit paid the closed account"
    );
    let c = contract(&tw, id);
    assert!(
        c.pending.is_some(),
        "the refused transition was not recorded as pending"
    );
    assert!(
        store(&tw)
            .escalations()
            .any(|e| e.subject == EscalationSubject::Contract(id)),
        "a refused forfeit was silent"
    );

    // The sweep retries it every fire, and spends the attempts rather than looping.
    for n in 1..=u32::from(mush_economy::MAX_SETTLE_ATTEMPTS) {
        sweep_at(&mut tw, 1_000 + i64::from(n) * 60);
    }
    assert_eq!(
        state(&tw, id),
        ContractState::Disputed,
        "the retries never gave up"
    );
    assert_eq!(
        held(&tw, r.j_acct),
        250,
        "the escrow was lost on the way to Disputed"
    );

    // …and `@contract/resolve` still ends it, because it was never terminal.
    ok(&mut tw, GOD, &format!("@contract/resolve {id}=refund"));
    assert_eq!(state(&tw, id), ContractState::Cancelled);
    assert_eq!(held(&tw, r.j_acct), 0, "the escrow was not recoverable");
    assert_eq!(bal(&tw, r.j_acct), 3_000);
}

/// M2 + M3 — a refused UNWIND leaves the contract live too, and `@contract/amend` is
/// refused while its escrow is still held.
#[test]
fn vf_m3_a_refused_unwind_stays_live_and_blocks_an_amendment() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    let id = offered_run(&mut tw, &r, 300);
    ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
    assert_eq!(held(&tw, r.j_acct), 300);

    // A consent surface that no longer matches its hash: `ContractAuth::unwinding`
    // is not bound, so the refund cannot release anything.
    let mut kv = fold_kv(&mut tw);
    patch(&mut kv, &format!("contract:{id}"), |v| {
        v["consent"]["terms"]["remainder_to"] = serde_json::json!(4242);
    });
    reboot(&mut tw, kv);
    assert_eq!(state(&tw, id), ContractState::Offered);
    assert_eq!(held(&tw, r.j_acct), 300, "the reboot lost the escrow");

    // E6a step 1 (A′): the Johnson's own acceptance would survive an amendment, so
    // nothing needs unwinding — and a tampered surface is refused outright rather
    // than laundered into a fresh draft under that acceptance. Nothing changes.
    let seen = err(&mut tw, r.johnson, &format!("@contract/amend {id}"));
    assert!(
        seen.contains("consent surface has changed"),
        "wrong refusal: {seen}"
    );
    let c = contract(&tw, id);
    assert_eq!(
        c.state,
        ContractState::Offered,
        "a failed amendment moved the state"
    );
    assert_eq!(
        c.amendments, 0,
        "a failed amendment burned a key generation"
    );
    assert_eq!(
        c.escrow.len(),
        1,
        "a failed amendment dropped the escrow from the record"
    );
    assert_eq!(
        held(&tw, r.j_acct),
        300,
        "a failed amendment stranded the escrow"
    );
    assert!(
        c.pending.is_none(),
        "a refused amendment parked a pending transition"
    );

    // The TERMINAL path keeps M2+M3's shape: a cancellation whose refund is refused
    // stays live, records `pending`, and pages — and an amendment cannot run over it.
    ok(&mut tw, r.johnson, &format!("@contract/cancel {id}"));
    let c = contract(&tw, id);
    assert_eq!(
        c.state,
        ContractState::Offered,
        "a refused unwind moved the state"
    );
    assert_eq!(held(&tw, r.j_acct), 300, "a refused unwind lost the escrow");
    assert!(
        c.pending.is_some(),
        "the refused unwind was not recorded as pending"
    );
    assert!(
        store(&tw)
            .escalations()
            .any(|e| e.subject == EscalationSubject::Contract(id)),
        "a refused unwind was silent"
    );
    // A second attempt says so plainly rather than trying again underneath.
    let seen = err(&mut tw, r.johnson, &format!("@contract/amend {id}"));
    assert!(seen.contains("has not landed"), "wrong refusal: {seen}");
}

/// M4 — settlement credits were membership-checked, not amount-checked.
#[test]
fn vf_m4_a_settle_plan_that_reroutes_the_split_is_refused() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    let id = offered_run(&mut tw, &r, 300);
    accept_all(&mut tw, &r, id);

    use mush_economy::AuthEval;
    let c = contract(&tw, id);
    let s = snap(&tw);
    let auth = ContractAuth::new(&c, s);
    let actor = PrincipalId::Object(r.johnson);
    let remainder = s.account(r.j_acct).expect("account");
    let payee = s.account(r.a_acct).expect("account");

    // The consented split pays 99/99/99 and 3 to the remainder. Those exact amounts,
    // and nothing else: a plan routing the whole 300 to `remainder_to` names an
    // account the terms DO name, which is precisely why membership was not enough.
    assert!(auth.authorize(&actor, remainder, Amount::from_minor(3)));
    assert!(!auth.authorize(&actor, remainder, Amount::from_minor(300)));
    assert!(!auth.authorize(&actor, remainder, Amount::from_minor(102)));
    assert!(auth.authorize(&actor, payee, Amount::from_minor(99)));
    assert!(!auth.authorize(&actor, payee, Amount::from_minor(198)));

    // A forfeit is the same rule with one payee: exactly the escrowed total.
    let forfeiting = ContractAuth::forfeiting(&c, s, r.b_acct);
    let target = s.account(r.b_acct).expect("account");
    assert!(forfeiting.authorize(&actor, target, Amount::from_minor(300)));
    assert!(!forfeiting.authorize(&actor, target, Amount::from_minor(299)));
}

/// M5 — an attestation could be overwritten, which moved an `After` anchor.
#[test]
fn vf_m5_a_re_attest_is_refused_and_the_after_anchor_does_not_move() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    ok(&mut tw, r.johnson, "@contract/draft Fence");
    let id = only_contract(&tw);
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/party {id}=#{}", r.a.0),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/pay {id}={} 100", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/split {id}={} 1000‰", r.a_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/remainder {id}={}", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/condition {id}=attest party:#{}", r.johnson.0),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/condition {id}=after 0 600"),
    );
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
    ok(&mut tw, r.a, &format!("@contract/accept {id}"));

    tw.set_now(1_000);
    ok(&mut tw, r.johnson, &format!("@contract/attest {id}=0"));
    let anchored_at = contract(&tw, id).satisfied[0]
        .as_ref()
        .expect("attested")
        .at;
    assert_eq!(anchored_at, 1_000);

    // A second attestation of the same condition is refused, so the consented delay
    // cannot be pushed forward one command at a time.
    tw.set_now(1_500);
    let seen = err(&mut tw, r.johnson, &format!("@contract/attest {id}=0"));
    assert!(seen.contains("already attested"), "wrong refusal: {seen}");
    assert_eq!(
        contract(&tw, id).satisfied[0]
            .as_ref()
            .expect("attested")
            .at,
        1_000,
        "a re-attest moved the anchor"
    );

    // …and it settles at the ORIGINAL delay, not 600 after the second attempt.
    sweep_at(&mut tw, 1_700);
    assert!(
        matches!(state(&tw, id), ContractState::Settled { .. }),
        "the delay did not mature"
    );
    assert_eq!(bal(&tw, r.a_acct), 100);
}

/// M6 — deposit return worked only for deposits made before activation.
#[test]
fn vf_m6_a_deposit_delivered_after_activation_still_returns_to_its_depositor() {
    let mut tw = boot();
    let buyer = tw.spawn_player("Buyer");
    let seller = tw.spawn_player("Seller");
    ok(&mut tw, GOD, "@instrument/register gold=0");
    ok(&mut tw, GOD, "@account/system faucet=gold");
    ok(&mut tw, GOD, &format!("@account/open #{}=gold", buyer.0));
    ok(&mut tw, GOD, &format!("@account/open #{}=gold", seller.0));
    let faucet = snap(&tw)
        .accounts()
        .find(|x| matches!(x.owner, PrincipalId::System(_)))
        .expect("faucet")
        .id;
    let buyer_acct = account_of(&tw, buyer);
    let seller_acct = account_of(&tw, seller);
    ok(
        &mut tw,
        GOD,
        &format!("@econ/mint {faucet}={buyer_acct} 500"),
    );
    let chest = thing(&mut tw, "chest");
    ok(
        &mut tw,
        GOD,
        &format!(
            "@vault/create escrowbox=container:#{} owner=#{}",
            chest.0, seller.0
        ),
    );
    let vault = snap(&tw).vaults().next().expect("vault").id;
    let widget = thing(&mut tw, "widget");
    tw.world_mut()
        .move_to(widget, seller)
        .expect("hand it to the seller");

    ok(&mut tw, seller, "@contract/draft Late");
    let id = only_contract(&tw);
    ok(
        &mut tw,
        seller,
        &format!("@contract/party {id}=#{}", buyer.0),
    );
    ok(
        &mut tw,
        buyer,
        &format!("@contract/pay {id}={buyer_acct} 200"),
    );
    ok(
        &mut tw,
        seller,
        &format!("@contract/split {id}={seller_acct} 1000‰"),
    );
    ok(
        &mut tw,
        seller,
        &format!("@contract/remainder {id}={seller_acct}"),
    );
    ok(
        &mut tw,
        seller,
        &format!("@contract/condition {id}=deposit #{} {vault}", widget.0),
    );
    ok(&mut tw, seller, &format!("@contract/offer {id}"));
    ok(&mut tw, seller, &format!("@contract/accept {id}"));
    ok(&mut tw, buyer, &format!("@contract/accept {id}"));
    assert_eq!(state(&tw, id), ContractState::Active);
    // Nothing was in the vault at activation, so the cache is empty…
    assert!(contract(&tw, id).deposits.is_empty());

    // …and the goods are delivered afterwards, which is the ordinary case.
    ok(
        &mut tw,
        seller,
        &format!("@vault/deposit #{}={vault}", widget.0),
    );
    let backing = snap(&tw)
        .vault(vault)
        .expect("vault")
        .backing
        .object()
        .expect("backing");
    assert_eq!(loc(&tw, widget), Some(backing));

    ok(&mut tw, GOD, &format!("@contract/cancel {id}"));
    assert_eq!(state(&tw, id), ContractState::Cancelled);
    assert_eq!(
        loc(&tw, widget),
        Some(seller),
        "a deposit delivered after activation was left in the vault with the lock gone"
    );
    assert_eq!(
        held(&tw, buyer_acct),
        0,
        "the refund did not release the escrow"
    );
    assert_eq!(bal(&tw, buyer_acct), 500);
}

/// M7 — `/roster open` let the builders re-sign a surface with no re-validation.
#[test]
fn vf_m7_roster_open_re_validates_the_surface_and_re_notifies() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    let id = offered_run(&mut tw, &r, 300);
    ok(&mut tw, r.johnson, &format!("@contract/roster {id}=open"));

    // A split that pays out more than is escrowed is refused where it was refused at
    // Offer, instead of being silently re-signed onto a live offer.
    let seen = err(
        &mut tw,
        r.johnson,
        &format!("@contract/split {id}={} 500", r.a_acct),
    );
    assert!(
        seen.contains("split is not feasible"),
        "wrong refusal: {seen}"
    );
    let c = contract(&tw, id);
    assert_eq!(
        c.consent.terms.split.len(),
        3,
        "a refused edit was still written"
    );
    assert!(
        c.consent_intact(),
        "a refused edit left the surface unsigned"
    );

    // A legitimate edit goes through, and the parties are told their offer moved.
    let seen = ok(
        &mut tw,
        r.johnson,
        &format!("@contract/split {id}={} 1000‰", r.a_acct),
    );
    assert!(
        seen.contains("offered terms have changed"),
        "the parties were not re-notified: {seen}"
    );
    assert_eq!(contract(&tw, id).consent.terms.split.len(), 1);
    ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
}

/// M8 — a consideration from someone who is not a party was offerable, and nobody
/// could ever fund it.
#[test]
fn vf_m8_a_non_party_payer_is_neither_offerable_nor_bootable() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    let id = offered_run(&mut tw, &r, 300);

    // The pure check, which is also what boot re-runs.
    let mut c = contract(&tw, id);
    c.consent.terms.consideration[0].from = PrincipalId::Object(r.watcher);
    assert!(
        matches!(
            mush_economy::check_structure(&c, &Default::default()),
            Err(mush_economy::ContractError::Infeasible(_))
        ),
        "a payer who is not a party passed the structural check"
    );
    assert!(mush_economy::check_structure(&contract(&tw, id), &Default::default()).is_ok());

    // …and a stored record carrying one is quarantined rather than left waiting for
    // money nobody can place.
    let mut kv = fold_kv(&mut tw);
    patch(&mut kv, &format!("contract:{id}"), |v| {
        v["consent"]["terms"]["consideration"][0]["from"] =
            serde_json::to_value(PrincipalId::Object(r.watcher)).expect("principal serializes");
    });
    reboot(&mut tw, kv);
    assert_eq!(
        state(&tw, id),
        ContractState::Disputed,
        "boot loaded an unfundable contract as live"
    );
    assert!(
        store(&tw)
            .escalations()
            .any(|e| e.subject == EscalationSubject::Contract(id))
    );
}

// ---- lens B: durability, boot, the sweep's contract step -------------------

/// The persisted value of one key, for a test that wants to put it back.
fn doc_of(kv: &[(SmolStr, serde_json::Value)], key: &str) -> serde_json::Value {
    kv.iter()
        .find(|(k, _)| k.as_str() == key)
        .map(|(_, v)| v.clone())
        .expect("no such doc")
}

/// B1 — boot disputed a contract whose terminal transition had already posted.
///
/// The transition is a post under the contract's own deterministic key followed by a
/// state write, so a lost doc write leaves the journal finished and the record still
/// claiming to be live. Boot resolves that by looking the key up, not by guessing.
#[test]
fn vf_b1_boot_finishes_a_transition_whose_doc_write_was_lost() {
    for verb in ["settle", "refund", "forfeit"] {
        let mut tw = boot();
        let r = run_rig(&mut tw, 3_000);
        ok(&mut tw, r.johnson, "@contract/draft Lost");
        let id = only_contract(&tw);
        ok(
            &mut tw,
            r.johnson,
            &format!("@contract/party {id}=#{}", r.a.0),
        );
        ok(
            &mut tw,
            r.johnson,
            &format!("@contract/pay {id}={} 300", r.j_acct),
        );
        ok(
            &mut tw,
            r.johnson,
            &format!("@contract/split {id}={} 1000‰", r.a_acct),
        );
        ok(
            &mut tw,
            r.johnson,
            &format!("@contract/remainder {id}={}", r.j_acct),
        );
        ok(
            &mut tw,
            r.johnson,
            &format!("@contract/condition {id}=attest party:#{}", r.johnson.0),
        );
        if verb == "forfeit" {
            ok(
                &mut tw,
                r.johnson,
                &format!("@contract/deadline {id}=600 forfeit={}", r.b_acct),
            );
        }
        ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
        ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
        ok(&mut tw, r.a, &format!("@contract/accept {id}"));
        assert_eq!(held(&tw, r.j_acct), 300);

        // The doc as it stood BEFORE the transition — the write that gets lost.
        let mut kv = fold_kv(&mut tw);
        let pre = doc_of(&kv, &format!("contract:{id}"));
        match verb {
            "settle" => {
                ok(&mut tw, r.johnson, &format!("@contract/attest {id}=0"));
            }
            "refund" => {
                ok(&mut tw, GOD, &format!("@contract/cancel {id}"));
            }
            _ => {
                sweep_at(&mut tw, 1_000);
            }
        }
        assert!(
            state(&tw, id).is_terminal(),
            "{verb}: the transition did not land"
        );
        merge(&mut kv, drain(&mut tw));
        patch(&mut kv, &format!("contract:{id}"), |v| *v = pre.clone());
        reboot(&mut tw, kv);

        let expected = match verb {
            "settle" => "settled",
            "refund" => "cancelled",
            _ => "expired",
        };
        let got = state(&tw, id);
        let ok_state = match expected {
            "settled" => matches!(got, ContractState::Settled { .. }),
            "cancelled" => got == ContractState::Cancelled,
            _ => got == ContractState::Expired,
        };
        assert!(
            ok_state,
            "{verb}: boot did not finish the transition, got {got:?}"
        );
        assert!(
            contract(&tw, id).pending.is_none(),
            "{verb}: pending survived the finish"
        );
        assert!(
            !store(&tw).is_frozen(),
            "{verb}: finishing a transition froze the economy"
        );
        assert!(
            !store(&tw)
                .escalations()
                .any(|e| e.subject == EscalationSubject::Contract(id)),
            "{verb}: boot paged a human about a contract that was already finished"
        );
        assert_eq!(
            held(&tw, r.j_acct),
            0,
            "{verb}: the escrow came back reserved"
        );
    }
}

/// B2 — a lost `accept` doc write orphaned the escrow forever.
#[test]
fn vf_b2_an_orphaned_escrow_hold_is_adopted_and_an_ended_contract_gives_it_back() {
    // (1) The contract doc rolls back; the hold survives, because it is journal
    // authority. Boot adopts it, and the contract refunds normally.
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    let id = offered_run(&mut tw, &r, 300);
    let mut kv = fold_kv(&mut tw);
    let pre = doc_of(&kv, &format!("contract:{id}"));
    ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
    assert_eq!(held(&tw, r.j_acct), 300);
    merge(&mut kv, drain(&mut tw));
    patch(&mut kv, &format!("contract:{id}"), |v| *v = pre.clone());
    reboot(&mut tw, kv.clone());

    let c = contract(&tw, id);
    assert_eq!(c.state, ContractState::Offered);
    assert_eq!(
        c.escrow.len(),
        1,
        "boot did not adopt the orphaned escrow hold"
    );
    assert_eq!(held(&tw, r.j_acct), 300, "boot moved money");
    ok(&mut tw, GOD, &format!("@contract/cancel {id}"));
    assert_eq!(
        held(&tw, r.j_acct),
        0,
        "the adopted escrow was not refundable"
    );
    assert_eq!(bal(&tw, r.j_acct), 3_000);

    // (2) The same orphan on a contract that has already ENDED. Boot cannot post, so
    // it adopts the hold, advances the key generation (the old refund key is spent)
    // and asks for the contract's own unwind; the sweep hands the money back.
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    let id = offered_run(&mut tw, &r, 300);
    let mut kv = fold_kv(&mut tw);
    let pre = doc_of(&kv, &format!("contract:{id}"));
    ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
    merge(&mut kv, drain(&mut tw));
    patch(&mut kv, &format!("contract:{id}"), |v| {
        *v = pre.clone();
        v["state"] = serde_json::json!("Cancelled");
    });
    reboot(&mut tw, kv);
    let c = contract(&tw, id);
    assert_eq!(c.state, ContractState::Cancelled);
    assert_eq!(
        c.escrow.len(),
        1,
        "boot did not adopt the orphan on an ended contract"
    );
    assert!(
        c.pending.is_some(),
        "boot did not schedule the ended contract's own unwind"
    );
    assert_eq!(held(&tw, r.j_acct), 300, "boot released money by itself");

    sweep_at(&mut tw, 100);
    assert_eq!(
        held(&tw, r.j_acct),
        0,
        "the sweep did not release the adopted escrow"
    );
    assert_eq!(bal(&tw, r.j_acct), 3_000);
    assert_eq!(
        state(&tw, id),
        ContractState::Cancelled,
        "the recovery moved the terminal state"
    );
    assert!(
        contract(&tw, id).pending.is_none(),
        "pending survived a landed unwind"
    );
}

/// B3 — a refused forfeit was silent: `dispute_with` returned at once on a terminal
/// contract, so the page never happened either.
#[test]
fn vf_b3_a_refused_forfeit_is_paged_once_and_stays_resolvable() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    ok(&mut tw, r.johnson, "@contract/draft Silent");
    let id = only_contract(&tw);
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/party {id}=#{}", r.a.0),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/pay {id}={} 250", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/split {id}={} 1000‰", r.a_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/remainder {id}={}", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/condition {id}=attest party:#{}", r.a.0),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/deadline {id}=600 forfeit={}", r.b_acct),
    );
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
    ok(&mut tw, r.a, &format!("@contract/accept {id}"));
    ok(&mut tw, r.b, &format!("@account/close {}", r.b_acct));

    for n in 0..4 {
        sweep_at(&mut tw, 1_000 + i64::from(n) * 60);
    }
    // One record for one decision, however many fires refused it.
    let records: Vec<_> = store(&tw)
        .escalations()
        .filter(|e| e.subject == EscalationSubject::Contract(id))
        .collect();
    assert_eq!(
        records.len(),
        1,
        "a refused forfeit minted an alert per fire: {records:?}"
    );
    assert!(
        records[0]
            .recommended
            .iter()
            .any(|c| c.contains("@contract/resolve")),
        "the page does not carry the command that ends it: {:?}",
        records[0].recommended
    );
    assert_eq!(
        held(&tw, r.j_acct),
        250,
        "the escrow was lost while the transition was refused"
    );
    assert_eq!(bal(&tw, r.b_acct), 0, "a closed account was paid");
}

/// B4 — step 3 starved: the per-fire cap was spent on records with nothing due, in id
/// order, so a high-id contract past its own deadline was never reached.
#[test]
fn vf_b4_a_high_id_contract_is_not_starved_by_a_wall_of_disputes() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    ok(&mut tw, r.johnson, "@contract/draft Victim");
    let id = only_contract(&tw);
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/party {id}=#{}", r.a.0),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/pay {id}={} 300", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/split {id}={} 1000‰", r.a_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/remainder {id}={}", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/condition {id}=attest party:#{}", r.a.0),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/deadline {id}=600 refund"),
    );
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));

    // The victim moves to a high id, and sixty-five long-disputed contracts sit in
    // front of it. Sixty-five is one more than the per-fire cap.
    let victim = mush_economy::ContractId(100);
    let mut kv = fold_kv(&mut tw);
    let doc = doc_of(&kv, &format!("contract:{id}"));
    kv.retain(|(k, _)| k.as_str() != format!("contract:{id}"));
    let mut moved = doc.clone();
    moved["id"] = serde_json::json!(victim.0);
    kv.push((SmolStr::new(format!("contract:{}", victim.0)), moved));
    for n in 1..=65u64 {
        let mut d = doc.clone();
        d["id"] = serde_json::json!(n);
        d["state"] = serde_json::json!("Disputed");
        d["disputed_at"] = serde_json::json!(0);
        d["consent"]["deadline"] = serde_json::Value::Null;
        kv.push((SmolStr::new(format!("contract:{n}")), d));
    }
    patch(&mut kv, "meta", |v| {
        v["next_contract_id"] = serde_json::json!(101)
    });
    reboot(&mut tw, kv);
    assert_eq!(snap(&tw).contracts().count(), 66);
    assert_eq!(state(&tw, victim), ContractState::Offered);

    // Two fires is the whole budget: the wall of disputes is work only while the
    // ladder has something to say, and the scan rotates so id order cannot starve.
    // Well past `DISPUTE_TTL_SECS`, so every one of the sixty-five disputes is work.
    sweep_at(&mut tw, 400_000);
    // The first fire spends its budget on the disputes (which is the old behaviour,
    // and legitimate: they had never been paged).
    assert_eq!(
        store(&tw).escalations().count(),
        64,
        "the first fire did not spend its budget on the wall of disputes"
    );
    assert_eq!(
        state(&tw, victim),
        ContractState::Offered,
        "the victim was reached early"
    );
    sweep_at(&mut tw, 400_060);
    assert_eq!(
        state(&tw, victim),
        ContractState::Expired,
        "a high-id contract was starved past its own deadline"
    );
}

/// B5 — the `Settled` check was existence-only, so a doc could claim another
/// contract's settlement and read as clean.
#[test]
fn vf_b5_a_settled_doc_claiming_another_contracts_transaction_is_drift() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 6_000);
    // Two contracts, both really settled.
    let first = offered_run(&mut tw, &r, 300);
    accept_all(&mut tw, &r, first);
    ok(&mut tw, r.johnson, &format!("@contract/attest {first}=0"));

    ok(&mut tw, r.johnson, "@contract/draft Second");
    let second = snap(&tw).contracts().map(|c| c.id).max().expect("second");
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/party {second}=#{}", r.a.0),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/pay {second}={} 200", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/split {second}={} 1000‰", r.a_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/remainder {second}={}", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/condition {second}=attest party:#{}", r.johnson.0),
    );
    ok(&mut tw, r.johnson, &format!("@contract/offer {second}"));
    ok(&mut tw, r.johnson, &format!("@contract/accept {second}"));
    ok(&mut tw, r.a, &format!("@contract/accept {second}"));
    ok(&mut tw, r.johnson, &format!("@contract/attest {second}=0"));
    let ContractState::Settled { transaction: other } = state(&tw, second) else {
        panic!("the second contract did not settle");
    };

    // The first contract's doc now names a transaction the journal HAS — and which
    // is not its settlement.
    let mut kv = fold_kv(&mut tw);
    patch(&mut kv, &format!("contract:{first}"), |v| {
        v["state"] = serde_json::json!({ "Settled": { "transaction": other.0 } });
    });
    reboot(&mut tw, kv);
    assert!(
        store(&tw).is_frozen(),
        "a settlement doc claiming another contract's transaction read as clean"
    );
    assert!(
        store(&tw)
            .escalations()
            .any(|e| e.subject == EscalationSubject::Contract(first)),
        "the freeze opened no companion record"
    );
}

/// B6 — boot trusted a stored surface, so a record that could never settle came back
/// live and held its escrow until a deadline it might not even have.
#[test]
fn vf_b6_boot_quarantines_a_stored_surface_that_could_never_settle() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    let id = offered_run(&mut tw, &r, 300);
    accept_all(&mut tw, &r, id);
    assert_eq!(state(&tw, id), ContractState::Active);
    assert_eq!(held(&tw, r.j_acct), 300);

    // Two `After` conditions anchored on each other: individually plausible, and
    // neither can ever be satisfied.
    let mut kv = fold_kv(&mut tw);
    patch(&mut kv, &format!("contract:{id}"), |v| {
        v["consent"]["conditions"] = serde_json::json!([
            { "After": { "condition": 1, "delay": 60, "clock": "RealUtc" } },
            { "After": { "condition": 0, "delay": 60, "clock": "RealUtc" } },
        ]);
        v["satisfied"] = serde_json::json!([null, null]);
    });
    reboot(&mut tw, kv);
    assert_eq!(
        state(&tw, id),
        ContractState::Disputed,
        "boot trusted a stored surface"
    );
    assert!(
        !store(&tw).is_frozen(),
        "one broken surface froze the whole economy"
    );
    assert!(
        store(&tw)
            .escalations()
            .any(|e| e.subject == EscalationSubject::Contract(id)),
        "boot quarantined a contract without paging"
    );
    // The escrow is untouched: quarantine reserves no value and moves none.
    assert_eq!(held(&tw, r.j_acct), 300, "quarantine moved the escrow");
    assert_eq!(bal(&tw, r.j_acct), 3_000);
}

// ======================================================================
// ---- E6a step 1: S1 — consent by authorship (DL-01 Option A′) ----
//
// An amendment authored by a party never voids that party's own acceptance: the
// author's escrow stands, every other acceptance is voided and refunded as E4b2
// did for all of them, and the re-offer re-stamps the survivor onto the amended
// surface. The tests below hold to 08f §2 item 1's five shapes plus the sweep's
// `Draft`-with-escrow expiry (M10) and M16's generation rule.
// ======================================================================

/// The run rig with the Johnson accepted and then amending: the A′ shape every
/// test in this section starts from.
fn payer_amended_run(tw: &mut TestWorld, r: &Run) -> (ContractId, Vec<HoldId>) {
    let id = offered_run(tw, r, 300);
    ok(tw, r.johnson, &format!("@contract/accept {id}"));
    let hold = contract(tw, id).escrow.clone();
    assert_eq!(hold.len(), 1, "the accept did not place one escrow hold");
    ok(tw, r.johnson, &format!("@contract/amend {id}"));
    (id, hold)
}

#[test]
fn e6s1_s1_a_payer_amend_keeps_its_acceptance_and_the_escrow_never_moves() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    let id = offered_run(&mut tw, &r, 300);
    ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
    ok(&mut tw, r.a, &format!("@contract/accept {id}"));
    let before = journal_len(&tw);
    let hold = contract(&tw, id).escrow.clone();

    let seen = ok(&mut tw, r.johnson, &format!("@contract/amend {id}"));
    assert!(
        seen.contains("is back to draft"),
        "unexpected reply: {seen}"
    );
    let c = contract(&tw, id);
    assert_eq!(c.state, ContractState::Draft);
    assert_eq!(
        c.acceptances
            .iter()
            .map(|a| a.party.clone())
            .collect::<Vec<_>>(),
        vec![PrincipalId::Object(r.johnson)],
        "the author's acceptance did not survive alone"
    );
    assert_eq!(c.escrow, hold, "the author's hold left the record");
    assert!(
        snap(&tw).hold(hold[0]).is_some_and(|h| h.status.is_held()),
        "the author's escrow was released"
    );
    assert_eq!(held(&tw, r.j_acct), 300, "the escrow moved");
    assert_eq!(bal(&tw, r.j_acct), 3_000);
    assert_eq!(
        journal_len(&tw),
        before,
        "an amendment that refunded nothing posted a transaction"
    );
    assert_eq!(
        c.amendments, 0,
        "an amendment that refunded nothing moved the key generation (M16)"
    );
    assert!(!c.roster_locked);

    // M16: re-splitting a listing thirty-odd times never exhausts MAX_AMENDMENTS,
    // because no refund ever posts.
    for n in 0..=mush_economy::MAX_AMENDMENTS {
        ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
        assert!(
            contract(&tw, id).accepted_by(&PrincipalId::Object(r.johnson)),
            "round {n}: the re-offer did not re-stamp the author's acceptance"
        );
        ok(&mut tw, r.johnson, &format!("@contract/amend {id}"));
        assert_eq!(held(&tw, r.j_acct), 300, "round {n}: the escrow moved");
    }
    assert_eq!(contract(&tw, id).amendments, 0);
    assert_eq!(journal_len(&tw), before);

    // The amendment rewrites the roster and the shares; the re-offer carries the
    // author's acceptance; the others accept; the Johnson never re-accepts.
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/party {id}=#{}", r.watcher.0),
    );
    let w_acct = account_of(&tw, r.watcher);
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/split {id}={} 500‰ {} 500‰", r.a_acct, w_acct),
    );
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    assert!(contract(&tw, id).accepted_by(&PrincipalId::Object(r.johnson)));
    for who in [r.a, r.b, r.c, r.watcher] {
        ok(&mut tw, who, &format!("@contract/accept {id}"));
    }
    assert_eq!(
        state(&tw, id),
        ContractState::Active,
        "the surviving acceptance did not count toward activation"
    );
    assert_eq!(
        contract(&tw, id).escrow,
        hold,
        "activation changed the escrow"
    );
    ok(&mut tw, r.johnson, &format!("@contract/attest {id}=0"));
    assert!(matches!(state(&tw, id), ContractState::Settled { .. }));
    assert_eq!(bal(&tw, r.a_acct), 150);
    assert_eq!(bal(&tw, w_acct), 150);
    assert_eq!(bal(&tw, r.j_acct), 2_700);
    assert_eq!(held(&tw, r.j_acct), 0);
    assert_eq!(
        journal_len(&tw),
        before + 1,
        "settlement was not the only post after the amendment"
    );
}

#[test]
fn e6s1_s1_a_fixer_issuers_amend_voids_the_payers_acceptance_and_refunds() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    let id = fixer_issued_run(&mut tw, &r, 300);
    ok(&mut tw, r.a, &format!("@contract/accept {id}"));
    assert_eq!(held(&tw, r.a_acct), 300);
    let before = journal_len(&tw);

    // The issuer is not the payer: today's behaviour, pinned — the payer's
    // acceptance is voided and her escrow comes back under the contract's own key.
    let seen = ok(&mut tw, r.johnson, &format!("@contract/amend {id}"));
    assert!(
        seen.contains("1 acceptance(s) were voided and refunded"),
        "unexpected reply: {seen}"
    );
    assert!(
        seen.contains("any acceptance of yours is void"),
        "the payer was not told: {seen}"
    );
    let c = contract(&tw, id);
    assert_eq!(c.state, ContractState::Draft);
    assert!(
        c.acceptances.is_empty(),
        "a non-author's acceptance survived"
    );
    assert!(c.escrow.is_empty(), "a released hold stayed on the record");
    assert_eq!(
        c.amendments, 1,
        "a posted refund did not move the generation"
    );
    assert_eq!(
        held(&tw, r.a_acct),
        0,
        "the payer's escrow was not released"
    );
    assert_eq!(bal(&tw, r.a_acct), 500);
    assert_eq!(journal_len(&tw), before + 1, "the refund was not one post");
    assert!(
        snap(&tw)
            .idempotency(&mush_economy::refund_key(id))
            .is_some(),
        "the refund did not post under contract/{{id}}/refund"
    );
}

#[test]
fn e6s1_s1_changing_your_own_escrowed_consideration_is_refused() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    let (id, hold) = payer_amended_run(&mut tw, &r);

    // At `/pay`: the amount a Held hold backs is not the issuer's to change.
    let seen = err(
        &mut tw,
        r.johnson,
        &format!("@contract/pay {id}={} 400", r.j_acct),
    );
    assert!(seen.contains("backed by escrow"), "wrong refusal: {seen}");
    let c = contract(&tw, id);
    assert_eq!(c.consent.terms.consideration[0].amount.minor(), 300);
    assert_eq!(c.escrow, hold);
    assert_eq!(held(&tw, r.j_acct), 300);
    // Re-stating the same amount is a no-op, not a change.
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/pay {id}={} 300", r.j_acct),
    );

    // At `/offer`: a NEW own consideration is money the surviving acceptance never
    // covered, so the surface cannot be re-offered under it.
    ok(
        &mut tw,
        GOD,
        &format!("@account/open #{}=nuyen", r.johnson.0),
    );
    let second = snap(&tw)
        .accounts()
        .filter(|a| a.owner == PrincipalId::Object(r.johnson))
        .map(|a| a.id)
        .max()
        .expect("a second wallet");
    assert_ne!(second, r.j_acct);
    let faucet = faucet(&tw);
    ok(&mut tw, GOD, &format!("@econ/mint {faucet}={second} 100"));
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/pay {id}={second} 50"),
    );
    let seen = err(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    assert!(
        seen.contains("no longer backed by escrow"),
        "wrong refusal: {seen}"
    );
    assert_eq!(state(&tw, id), ContractState::Draft);
    assert_eq!(held(&tw, r.j_acct), 300, "a refused offer moved the escrow");
    assert_eq!(held(&tw, second), 0);
}

#[test]
fn e6s1_s1_cancel_after_an_a_prime_amend_refunds_the_authors_escrow() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    let (id, hold) = payer_amended_run(&mut tw, &r);
    let before = journal_len(&tw);
    ok(&mut tw, r.johnson, &format!("@contract/cancel {id}"));
    assert_eq!(state(&tw, id), ContractState::Cancelled);
    assert_eq!(held(&tw, r.j_acct), 0, "the cancellation kept the escrow");
    assert_eq!(bal(&tw, r.j_acct), 3_000);
    assert_eq!(journal_len(&tw), before + 1);
    assert!(
        snap(&tw).hold(hold[0]).is_some_and(|h| !h.status.is_held()),
        "the author's hold is still held"
    );
    // No generation moved at the amendment, so the refund posted under the
    // generation-0 key.
    assert!(
        snap(&tw)
            .idempotency(&mush_economy::refund_key(id))
            .is_some(),
        "the refund did not post under generation 0"
    );
}

#[test]
fn e6s1_s1_the_sweep_expires_a_draft_holding_escrow_past_its_deadline() {
    let mut tw = boot();
    tw.set_now(1_000);
    let r = run_rig(&mut tw, 3_000);
    ok(&mut tw, r.johnson, "@contract/draft Windowed");
    let id = only_contract(&tw);
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/party {id}=#{}", r.a.0),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/pay {id}={} 300", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/split {id}={} 1000‰", r.a_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/remainder {id}={}", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/condition {id}=attest party:#{}", r.johnson.0),
    );
    // Ten days out — past the draft TTL, so the two rules can be told apart.
    let window = 10 * 86_400;
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/deadline {id}={window} dispute"),
    );
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
    ok(&mut tw, r.johnson, &format!("@contract/amend {id}"));
    assert_eq!(state(&tw, id), ContractState::Draft);
    assert_eq!(held(&tw, r.j_acct), 300);

    // Past the draft TTL but inside the window: NOT collected, NOT expired.
    sweep_at(&mut tw, 1_000 + mush_economy::DRAFT_TTL_SECS + 3_600);
    assert_eq!(
        state(&tw, id),
        ContractState::Draft,
        "a draft holding escrow was collected or expired inside its window"
    );
    assert_eq!(held(&tw, r.j_acct), 300);

    // Past the window: unwound, escrow back — a Draft is never bound, so `dispute`
    // on expiry refunds.
    sweep_at(&mut tw, 1_000 + window + 3_600);
    assert_eq!(
        state(&tw, id),
        ContractState::Expired,
        "the sweep did not expire a draft holding escrow past its deadline"
    );
    assert_eq!(held(&tw, r.j_acct), 0, "expiry kept the escrow");
    assert_eq!(bal(&tw, r.j_acct), 3_000);
}

#[test]
fn e6s1_s1_reboot_loads_the_a_prime_draft_with_its_escrow_and_acceptance() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    let (id, hold) = payer_amended_run(&mut tw, &r);
    let kv = fold_kv(&mut tw);
    reboot(&mut tw, kv);
    assert!(!store(&tw).is_frozen(), "the reboot froze the economy");
    let c = contract(&tw, id);
    assert_eq!(c.state, ContractState::Draft);
    assert_eq!(
        c.acceptances.len(),
        1,
        "the author's acceptance did not survive the reboot"
    );
    assert_eq!(c.escrow, hold, "boot's orphan pass changed the escrow list");
    assert_eq!(c.amendments, 0);
    assert_eq!(
        held(&tw, r.j_acct),
        300,
        "the escrow changed across the reboot"
    );
    assert!(
        !store(&tw)
            .escalations()
            .any(|e| e.subject == EscalationSubject::Contract(id)),
        "boot paged about a draft holding its author's escrow"
    );
    // And the shape still works after the restart: re-offer, the others accept, settle.
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    for who in [r.a, r.b, r.c] {
        ok(&mut tw, who, &format!("@contract/accept {id}"));
    }
    assert_eq!(state(&tw, id), ContractState::Active);
    ok(&mut tw, r.johnson, &format!("@contract/attest {id}=0"));
    assert!(matches!(state(&tw, id), ContractState::Settled { .. }));
    assert_eq!(held(&tw, r.j_acct), 0);
    assert_eq!(bal(&tw, r.a_acct), 99);
}

// ======================================================================
// ---- E6a step 1: S2 — the rolling approval window (D-E6-21, DL-10) ----
//
// A per-transaction threshold is not a budget: an officer with
// `economy.account.post` on an org treasury could post ≤-threshold debits forever
// (08f review M3). `ApprovalRule.window { total, period_secs }` escalates the debit
// that would take the account's trailing-period spend — negative lines plus placed
// holds, a refund never subtracted — past the total, on the threshold's own path.
// ======================================================================

/// An org treasury of 1000 under `threshold 100` with a window of 400 per hour,
/// and Quinn holding `economy.account.post` on its compartment.
fn windowed_treasury(tw: &mut TestWorld) -> Org {
    tw.set_now(1_000);
    let o = org_rig(tw);
    ok(
        tw,
        GOD,
        &format!(
            "@org/grant #{}=Ironhold economy.account.post scope=#{}",
            o.member.0, o.scope.0
        ),
    );
    let seen = ok(
        tw,
        GOD,
        &format!(
            "@account/approval {}=100 threshold:1 window=400/3600",
            o.acct
        ),
    );
    assert!(seen.contains("inside any trailing 3600 s"), "{seen}");
    o
}

/// One ≤-threshold debit from the treasury at wall-clock `t`.
fn drip(tw: &mut TestWorld, o: &Org, t: i64) -> String {
    tw.set_now(t);
    ok(
        tw,
        o.member,
        &format!("@econ/pay {}={} 100", o.acct, o.payee_acct),
    )
}

#[test]
fn e6s1_s2_the_fifth_sub_threshold_debit_inside_the_window_needs_approval() {
    let mut tw = boot();
    let o = windowed_treasury(&mut tw);
    for t in [1_000, 1_010, 1_020, 1_030] {
        let seen = drip(&mut tw, &o, t);
        assert!(seen.contains("Paid: transaction"), "at {t}: {seen}");
    }
    assert_eq!(bal(&tw, o.acct), 600);
    // The fifth ≤-threshold debit would take the hour's spend to 500 > 400.
    let seen = drip(&mut tw, &o, 1_040);
    assert!(seen.contains("Approval required"), "{seen}");
    assert_eq!(bal(&tw, o.acct), 600, "an unapproved debit posted");
    assert!(
        snap(&tw).approvals().any(|r| r.accounts.contains(&o.acct)),
        "the window fired without minting the threshold's own request"
    );
    // Once the window has rolled past the first four, the same post needs nothing.
    let seen = drip(&mut tw, &o, 1_000 + 3_600 + 31);
    assert!(seen.contains("Paid: transaction"), "{seen}");
    assert_eq!(bal(&tw, o.acct), 500);
    // `@account/show` says so.
    let seen = ok(&mut tw, GOD, &format!("@account/show {}", o.acct));
    assert!(seen.contains("rolling window: 400 per 3600 s"), "{seen}");
}

#[test]
fn e6s1_s2_a_placement_counts_and_a_refund_never_reopens_the_window() {
    let mut tw = boot();
    let o = windowed_treasury(&mut tw);
    for t in [1_000, 1_010, 1_020] {
        drip(&mut tw, &o, t);
    }
    // A placed hold is a promise to pay and counts like a debit (D-E4-11).
    tw.set_now(1_030);
    ok(
        &mut tw,
        o.member,
        &format!("@hold/place {}=100 window probe", o.acct),
    );
    assert_eq!(held(&tw, o.acct), 100);
    let seen = drip(&mut tw, &o, 1_040);
    assert!(
        seen.contains("Approval required"),
        "a placement did not count toward the window: {seen}"
    );
    // Releasing the hold gives nothing back to the window; neither does reversing
    // a posted debit — conservative by design.
    let hold = snap(&tw)
        .holds_of(o.acct)
        .find(|h| h.status.is_held())
        .expect("the probe hold")
        .id;
    ok(&mut tw, o.member, &format!("@hold/release #{hold}"));
    assert_eq!(held(&tw, o.acct), 0);
    let last = store(&tw)
        .transactions()
        .filter(|t| {
            t.lines
                .iter()
                .any(|l| l.account == o.acct && l.delta.is_negative())
        })
        .map(|t| t.id)
        .max()
        .expect("a posted debit");
    ok(&mut tw, GOD, &format!("@econ/reverse {last}"));
    assert_eq!(
        bal(&tw, o.acct),
        800,
        "the reversal did not credit the treasury"
    );
    let seen = drip(&mut tw, &o, 1_050);
    assert!(
        seen.contains("Approval required"),
        "a release or a reversal re-opened the window: {seen}"
    );
    assert_eq!(bal(&tw, o.acct), 800);
}

#[test]
fn e6s1_s2_the_window_tail_survives_a_reboot() {
    // A personal wallet: the reboot rig reloads the economy namespace alone, and an
    // org compartment would not come back with it.
    let mut tw = boot();
    tw.set_now(1_000);
    let r = run_rig(&mut tw, 3_000);
    ok(
        &mut tw,
        GOD,
        &format!(
            "@account/approval {}=100 threshold:1 window=400/3600",
            r.j_acct
        ),
    );
    for t in [1_000, 1_010, 1_020, 1_030] {
        tw.set_now(t);
        let seen = ok(
            &mut tw,
            r.johnson,
            &format!("@econ/pay {}={} 100", r.j_acct, r.a_acct),
        );
        assert!(seen.contains("Paid: transaction"), "at {t}: {seen}");
    }
    let kv = fold_kv(&mut tw);
    reboot(&mut tw, kv);
    assert!(!store(&tw).is_frozen());
    assert_eq!(
        store(&tw).spent_in_window(r.j_acct, 1_040, 3_600).minor(),
        400,
        "replay did not rebuild the window tail"
    );
    tw.set_now(1_040);
    let seen = ok(
        &mut tw,
        r.johnson,
        &format!("@econ/pay {}={} 100", r.j_acct, r.a_acct),
    );
    assert!(
        seen.contains("Approval required"),
        "the window forgot the pre-reboot spend: {seen}"
    );
    assert_eq!(bal(&tw, r.j_acct), 2_600);
}

#[test]
fn e6s1_s2_window_off_restores_threshold_only() {
    let mut tw = boot();
    let o = windowed_treasury(&mut tw);
    for t in [1_000, 1_010, 1_020, 1_030] {
        drip(&mut tw, &o, t);
    }
    let seen = drip(&mut tw, &o, 1_040);
    assert!(seen.contains("Approval required"), "{seen}");
    // A bare `window=off` keeps the threshold and the policy and drops the budget.
    let seen = ok(
        &mut tw,
        GOD,
        &format!("@account/approval {}=window=off", o.acct),
    );
    assert!(seen.contains("rolling window is off"), "{seen}");
    let rule = snap(&tw)
        .account(o.acct)
        .expect("account")
        .approval
        .clone()
        .expect("the rule stands");
    assert_eq!(rule.threshold.minor(), 100);
    assert!(rule.window.is_none());
    let seen = drip(&mut tw, &o, 1_050);
    assert!(seen.contains("Paid: transaction"), "{seen}");
    // The threshold alone still escalates an over-threshold debit.
    tw.set_now(1_060);
    let seen = ok(
        &mut tw,
        o.member,
        &format!("@econ/pay {}={} 101", o.acct, o.payee_acct),
    );
    assert!(seen.contains("Approval required"), "{seen}");
    // A malformed or out-of-band window is refused and changes nothing.
    err(
        &mut tw,
        GOD,
        &format!("@account/approval {}=window=400/0", o.acct),
    );
    err(
        &mut tw,
        GOD,
        &format!(
            "@account/approval {}=window=400/{}",
            o.acct,
            mush_economy::MAX_APPROVAL_WINDOW_SECS + 1
        ),
    );
    assert!(
        snap(&tw)
            .account(o.acct)
            .expect("account")
            .approval
            .as_ref()
            .is_some_and(|r| r.window.is_none())
    );
}

// ======================================================================
// ---- E6a step 1: S4 — the programmatic contract seams ----
//
// `offer_contract` / `accept_contract` / `amend_contract` (08f §2 item 4): a
// ruleset drafts, offers, accepts and amends through crate APIs and never types
// a `@contract` command. Driven through an `@e6` probe, because the seams take a
// `Ctx` and are deliberately not verbs.
// ======================================================================

use std::sync::Mutex;

use mush_core::module::{GameModule, ModError, Registrar};
use mush_core::{ArgShape, CmdError, CommandSpec, Ctx};
use mush_economy::{
    AcceptOutcome, AmendReoffer, AttestorSet, ConsiderationSpec, ContractAmendment, ContractError,
    ContractRoles, ContractSpec, EconomicVisibility, ExpenseTreatment, Expiry, OutcomeTerms,
    PayoutShare, Share, TypedCondition,
};

#[derive(Default)]
struct E6State {
    spec: Option<ContractSpec>,
    amend: Option<ContractAmendment>,
    offered: Option<Result<ContractId, ContractError>>,
    accepted: Option<Result<AcceptOutcome, ContractError>>,
    amended: Option<Result<AmendReoffer, ContractError>>,
    cancelled: Option<Result<bool, ContractError>>,
}

#[derive(Clone, Default)]
struct E6(Arc<Mutex<E6State>>);

impl E6 {
    fn stage_offer(&self, spec: ContractSpec) {
        self.0.lock().expect("probe lock").spec = Some(spec);
    }
    fn offered(&self) -> Result<ContractId, ContractError> {
        self.0
            .lock()
            .expect("probe lock")
            .offered
            .clone()
            .expect("an offer outcome was recorded")
    }
    fn accepted(&self) -> Result<AcceptOutcome, ContractError> {
        self.0
            .lock()
            .expect("probe lock")
            .accepted
            .clone()
            .expect("an accept outcome was recorded")
    }
    fn stage_amend(&self, amend: ContractAmendment) {
        self.0.lock().expect("probe lock").amend = Some(amend);
    }
    fn amended(&self) -> Result<AmendReoffer, ContractError> {
        self.0
            .lock()
            .expect("probe lock")
            .amended
            .clone()
            .expect("an amend outcome was recorded")
    }
    fn cancelled(&self) -> Result<bool, ContractError> {
        self.0
            .lock()
            .expect("probe lock")
            .cancelled
            .clone()
            .expect("a cancel outcome was recorded")
    }
}

struct E6Mod(E6);

impl GameModule for E6Mod {
    fn name(&self) -> &'static str {
        "e6probe"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(mush_core::CommandCompatibility::EconomyCarveOut);
        let st = self.0.clone();
        reg.command(
            CommandSpec {
                name: "@e6".into(),
                shape: ArgShape::EqPair,
                switches: ["offer", "accept", "amend", "cancel", "notice"]
                    .iter()
                    .copied()
                    .map(SmolStr::new)
                    .collect(),
                help: "E6a step 1 probe: offer a staged spec / accept <id> / amend <id> with the \
                       staged amendment, through the programmatic seams / notice <id> renders \
                       the exported notice class"
                    .into(),
                ..CommandSpec::default()
            },
            Arc::new(move |ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                let id = || -> ContractId {
                    ContractId(
                        ctx.args
                            .lhs()
                            .trim()
                            .trim_start_matches('#')
                            .parse()
                            .unwrap_or(u64::MAX),
                    )
                };
                if ctx.switches.has("offer") {
                    let spec =
                        st.0.lock()
                            .expect("probe lock")
                            .spec
                            .take()
                            .expect("stage a spec first");
                    let outcome = mush_economy::offer_contract(ctx, spec);
                    st.0.lock().expect("probe lock").offered = Some(outcome);
                } else if ctx.switches.has("accept") {
                    let outcome = mush_economy::accept_contract(ctx, id());
                    st.0.lock().expect("probe lock").accepted = Some(outcome);
                } else if ctx.switches.has("amend") {
                    let amend =
                        st.0.lock()
                            .expect("probe lock")
                            .amend
                            .take()
                            .expect("stage an amendment first");
                    let outcome = mush_economy::amend_contract(ctx, id(), amend);
                    let fail = ctx.args.rhs().is_some_and(|r| r.trim() == "fail");
                    st.0.lock().expect("probe lock").amended = Some(outcome);
                    // E6a step 1 verify fold (R1): a ruleset handler that fails for a
                    // reason of its own AFTER a seam post. Dispatch truncates a failed
                    // handler's deferred queue, so this is the exact shape that used to
                    // throw a landed transaction's writes away.
                    if fail {
                        return Err(CmdError::msg("the ruleset changed its mind"));
                    }
                } else if ctx.switches.has("cancel") {
                    let outcome = mush_economy::cancel_contract(ctx, id());
                    st.0.lock().expect("probe lock").cancelled = Some(outcome);
                } else if ctx.switches.has("notice") {
                    // S8: the exported notice class, rendered to the actor exactly as
                    // a run board's `+run/show` would render it.
                    let id = id();
                    let c = ctx
                        .w
                        .module_state::<EconomyStore>()
                        .and_then(|s| s.snapshot().contract(id).cloned())
                        .ok_or_else(|| CmdError::msg("No such contract."))?;
                    for line in mush_economy::contract_notice(ctx, &c) {
                        let actor = ctx.actor;
                        ctx.out.sys_to_actor(actor, "probe", line);
                    }
                }
                Ok(())
            }),
        )?;
        Ok(())
    }
}

fn boot_e6() -> (TestWorld, E6) {
    let e6 = E6::default();
    let tw = TestWorld::with_modules(vec![
        Box::new(
            mush_org::OrgModule::new()
                .with_capabilities(vocabulary())
                .with_scope_use(vec![Arc::new(mush_economy::EconomyScopeUse)]),
        ),
        Box::new(mush_economy::EconomyModule::with_custody(vec![])),
        Box::new(E6Mod(e6.clone())),
    ]);
    (tw, e6)
}

fn exact(to: AccountId, minor: i64) -> PayoutShare {
    PayoutShare {
        to,
        share: Share::Exact(Amount::from_minor(minor)),
    }
}

fn row(name: &str, when: Vec<usize>, payouts: Vec<PayoutShare>) -> OutcomeTerms {
    OutcomeTerms {
        name: SmolStr::new(name),
        when,
        payouts,
        expense_treatment: ExpenseTreatment::None,
        asset_consequences: Vec::new(),
    }
}

/// A run-shaped spec (08f §3.1): one consideration, two attestation conditions
/// deciding success (0) or failure (1), the payer as adjudicator, `Dispute` on
/// expiry, and whatever parties and rows the caller names.
fn run_spec(
    name: &str,
    payer: ObjId,
    pay: (AccountId, i64),
    parties: Vec<PrincipalId>,
    outcomes: Vec<OutcomeTerms>,
) -> ContractSpec {
    ContractSpec {
        name: SmolStr::new(name),
        scope: None,
        parties,
        considerations: vec![ConsiderationSpec {
            account: pay.0,
            amount: Amount::from_minor(pay.1),
        }],
        outcomes,
        remainder_to: pay.0,
        conditions: vec![
            TypedCondition::Attestation {
                by: AttestorSet::Party(PrincipalId::Object(payer)),
            },
            TypedCondition::Attestation {
                by: AttestorSet::Party(PrincipalId::Object(payer)),
            },
        ],
        adjudicator: Some(AttestorSet::Party(PrincipalId::Object(payer))),
        evidence: vec![mush_economy::EvidenceKind::Attestation],
        deadline: None,
        on_expiry: Expiry::Dispute,
        visibility: EconomicVisibility::Parties,
        roles: ContractRoles::default(),
        deterministic: false,
        deliverables: Vec::new(),
    }
}

#[test]
fn e6s1_s4_a_run_shaped_contract_goes_end_to_end_through_the_seams_only() {
    let (mut tw, e6) = boot_e6();
    let r = run_rig(&mut tw, 3_000);
    let fixer = r.watcher;
    let fixer_acct = account_of(&tw, fixer);
    // The board post: the payer is the issuer (D-E6-19), the fixer is a fee row and
    // the representative, and the crew's share sits provisionally on the payer's own
    // account until the crew is known (08f §3.1).
    let mut spec = run_spec(
        "Extraction",
        r.johnson,
        (r.j_acct, 1_000),
        Vec::new(),
        vec![
            row(
                "success",
                vec![0],
                vec![exact(r.j_acct, 800), exact(fixer_acct, 200)],
            ),
            row("failure", vec![1], vec![exact(r.j_acct, 1_000)]),
        ],
    );
    spec.roles.representative = Some(PrincipalId::Object(fixer));
    e6.stage_offer(spec);
    ok(&mut tw, r.johnson, "@e6/offer");
    let id = e6.offered().expect("the seam refused a well-formed run");
    let c = contract(&tw, id);
    assert_eq!(c.state, ContractState::Offered);
    assert_eq!(c.issuer, PrincipalId::Object(r.johnson));
    assert_eq!(c.roles.author, Some(PrincipalId::Object(r.johnson)));
    assert_eq!(c.roles.payer, Some(PrincipalId::Object(r.johnson)));
    assert_eq!(c.roles.representative, Some(PrincipalId::Object(fixer)));
    assert_eq!(
        snap(&tw).contracts().count(),
        1,
        "the offer left a draft behind"
    );

    // The payer's own accept places the escrow (answer 24: no listing before its
    // escrow). With nobody else on the surface it is also the LAST acceptance, so
    // the listing is a one-party contract in force — bound to nobody but its author.
    ok(&mut tw, r.johnson, &format!("@e6/accept {id}"));
    let Ok(AcceptOutcome::Accepted { escrow, activated }) = e6.accepted() else {
        panic!("the payer's accept was refused: {:?}", e6.accepted())
    };
    assert_eq!(escrow.len(), 1);
    assert!(
        activated,
        "a one-party contract did not activate at its author's accept"
    );
    assert_eq!(state(&tw, id), ContractState::Active);
    assert_eq!(held(&tw, r.j_acct), 1_000);
    let before = journal_len(&tw);

    // `+run/crew accept`: an A′ amendment adds the runners and rewrites every row to
    // their accounts, then re-offers — the payer's acceptance and escrow stand, and
    // an author-only contract in force re-opens without unwinding anything.
    e6.stage_amend(ContractAmendment {
        add_parties: vec![PrincipalId::Object(r.a), PrincipalId::Object(r.b)],
        outcomes: Some(vec![
            row(
                "success",
                vec![0],
                vec![
                    exact(r.a_acct, 400),
                    exact(r.b_acct, 400),
                    exact(fixer_acct, 200),
                ],
            ),
            row("failure", vec![1], vec![exact(r.j_acct, 1_000)]),
        ]),
        on_expiry: Some(Expiry::Dispute),
        reoffer: true,
    });
    ok(&mut tw, r.johnson, &format!("@e6/amend {id}"));
    e6.amended().expect("the crewing amendment was refused");
    let c = contract(&tw, id);
    assert_eq!(
        c.state,
        ContractState::Offered,
        "the amendment did not re-offer"
    );
    assert_eq!(c.consent.parties.len(), 3);
    assert_eq!(c.escrow, escrow, "the amendment moved the escrow");
    assert!(
        c.accepted_by(&PrincipalId::Object(r.johnson)),
        "the payer's acceptance did not survive the crewing amendment"
    );
    assert_eq!(
        c.amendments, 0,
        "an amendment that refunded nothing burned a generation"
    );
    assert_eq!(held(&tw, r.j_acct), 1_000);
    assert_eq!(
        journal_len(&tw),
        before,
        "the crewing amendment posted something"
    );

    // The runners accept through the seam; the last one activates.
    ok(&mut tw, r.a, &format!("@e6/accept {id}"));
    assert!(matches!(
        e6.accepted(),
        Ok(AcceptOutcome::Accepted {
            activated: false,
            ..
        })
    ));
    ok(&mut tw, r.b, &format!("@e6/accept {id}"));
    assert!(matches!(
        e6.accepted(),
        Ok(AcceptOutcome::Accepted {
            activated: true,
            ..
        })
    ));
    assert_eq!(state(&tw, id), ContractState::Active);

    // The adjudicator (the payer) attests success; the row pays exactly what it
    // names and no remainder reaches the fixer.
    ok(&mut tw, r.johnson, &format!("@contract/attest {id}=0"));
    assert!(matches!(state(&tw, id), ContractState::Settled { .. }));
    assert_eq!(bal(&tw, r.a_acct), 400);
    assert_eq!(bal(&tw, r.b_acct), 400);
    assert_eq!(bal(&tw, fixer_acct), 200);
    assert_eq!(bal(&tw, r.j_acct), 2_000);
    assert_eq!(held(&tw, r.j_acct), 0);
    assert_eq!(journal_len(&tw), before + 1);
}

#[test]
fn e6s1_s4_a_fixer_issuers_amend_refunds_the_payer_through_the_seam() {
    let (mut tw, e6) = boot_e6();
    let r = run_rig(&mut tw, 0);
    let faucet = faucet(&tw);
    ok(
        &mut tw,
        GOD,
        &format!("@econ/mint {faucet}={} 500", r.a_acct),
    );
    // Staff as the fixer-issuer (they may view the payer's wallet): the payer is a
    // party who is not the author.
    let mut spec = run_spec(
        "Brokered",
        GOD,
        (r.a_acct, 300),
        vec![PrincipalId::Object(r.a)],
        vec![
            row("success", vec![0], vec![exact(r.b_acct, 300)]),
            row("failure", vec![1], vec![exact(r.a_acct, 300)]),
        ],
    );
    spec.roles.author = Some(PrincipalId::Object(GOD));
    e6.stage_offer(spec);
    ok(&mut tw, GOD, "@e6/offer");
    let id = e6.offered().expect("offered");
    ok(&mut tw, r.a, &format!("@e6/accept {id}"));
    e6.accepted().expect("the payer accepted");
    assert_eq!(held(&tw, r.a_acct), 300);
    let before = journal_len(&tw);

    e6.stage_amend(ContractAmendment::default());
    ok(&mut tw, GOD, &format!("@e6/amend {id}"));
    e6.amended().expect("amended");
    let c = contract(&tw, id);
    assert_eq!(c.state, ContractState::Draft);
    assert!(
        c.acceptances.is_empty(),
        "a non-author's acceptance survived"
    );
    assert_eq!(c.amendments, 1);
    assert_eq!(
        held(&tw, r.a_acct),
        0,
        "the payer's escrow was not refunded"
    );
    assert_eq!(bal(&tw, r.a_acct), 500);
    assert_eq!(journal_len(&tw), before + 1);
}

#[test]
fn e6s1_s4_accept_by_a_non_party_is_refused_with_one_answer() {
    let (mut tw, e6) = boot_e6();
    let r = run_rig(&mut tw, 3_000);
    let mut spec = run_spec(
        "Closed",
        r.johnson,
        (r.j_acct, 100),
        vec![PrincipalId::Object(r.a)],
        vec![
            row("success", vec![0], vec![exact(r.a_acct, 100)]),
            row("failure", vec![1], vec![exact(r.j_acct, 100)]),
        ],
    );
    // Bex may VIEW it (she is the adjudicator) and is still not a party.
    spec.adjudicator = Some(AttestorSet::Party(PrincipalId::Object(r.b)));
    e6.stage_offer(spec);
    ok(&mut tw, r.johnson, "@e6/offer");
    let id = e6.offered().expect("offered");
    for outsider in [r.watcher, r.b] {
        ok(&mut tw, outsider, &format!("@e6/accept {id}"));
        assert_eq!(
            e6.accepted(),
            Err(ContractError::NoStanding),
            "a non-party's accept was not refused with the one answer"
        );
    }
    // A contract that does not exist answers the same.
    ok(&mut tw, r.a, "@e6/accept 4242");
    assert_eq!(e6.accepted(), Err(ContractError::NoStanding));
    assert!(contract(&tw, id).acceptances.is_empty());
    assert_eq!(held(&tw, r.j_acct), 0);
}

#[test]
fn e6s1_s4_a_refused_offer_leaves_no_draft_behind() {
    let (mut tw, e6) = boot_e6();
    let r = run_rig(&mut tw, 3_000);
    // Alder's wallet is not the Johnson's to name (A1's rule (a)): the account
    // answers as if it did not exist, and no contract is allocated or written.
    let spec = run_spec(
        "Probe",
        r.johnson,
        (r.a_acct, 100),
        vec![PrincipalId::Object(r.a)],
        vec![row("success", vec![0], vec![exact(r.j_acct, 100)])],
    );
    e6.stage_offer(spec);
    ok(&mut tw, r.johnson, "@e6/offer");
    assert_eq!(e6.offered(), Err(ContractError::NoSuchAccount(r.a_acct)));
    assert_eq!(
        snap(&tw).contracts().count(),
        0,
        "a refused offer left a draft"
    );
    // A deterministic condition is refused unless the spec opts in (D-E6-20).
    let mut spec = run_spec(
        "Clocked",
        r.johnson,
        (r.j_acct, 100),
        vec![PrincipalId::Object(r.a)],
        vec![row("success", vec![0], vec![exact(r.a_acct, 100)])],
    );
    spec.conditions.push(TypedCondition::Before {
        deadline: mush_economy::EconomicDeadline {
            clock: mush_economy::EconomicClock::RealUtc,
            at: 10_000,
        },
    });
    e6.stage_offer(spec);
    ok(&mut tw, r.johnson, "@e6/offer");
    assert_eq!(e6.offered(), Err(ContractError::EvidenceNotPermitted(2)));
    assert_eq!(snap(&tw).contracts().count(), 0);
}

#[test]
fn e6s1_s4_needs_approval_on_an_org_payers_accept_leaves_no_acceptance() {
    let (mut tw, e6) = boot_e6();
    let o = org_rig(&mut tw);
    ok(
        &mut tw,
        GOD,
        &format!("@account/approval {}=100 threshold:1", o.acct),
    );
    for cap in ["contract.accept", "economy.account.post"] {
        ok(
            &mut tw,
            GOD,
            &format!(
                "@org/grant #{}=Ironhold {cap} scope=#{}",
                o.member.0, o.scope.0
            ),
        );
    }
    // Staff post an org-issued run on the compartment.
    let spec = ContractSpec {
        name: SmolStr::new("Supply"),
        scope: Some(o.scope),
        parties: vec![PrincipalId::Object(o.payee)],
        considerations: vec![ConsiderationSpec {
            account: o.acct,
            amount: Amount::from_minor(400),
        }],
        outcomes: vec![row("success", vec![0], vec![exact(o.payee_acct, 400)])],
        remainder_to: o.acct,
        conditions: vec![TypedCondition::Attestation {
            by: AttestorSet::Staff,
        }],
        adjudicator: None,
        evidence: Vec::new(),
        deadline: None,
        on_expiry: Expiry::Refund,
        visibility: EconomicVisibility::Parties,
        roles: ContractRoles::default(),
        deterministic: false,
        deliverables: Vec::new(),
    };
    e6.stage_offer(spec);
    ok(&mut tw, GOD, "@e6/offer");
    let id = e6.offered().expect("offered");
    assert_eq!(contract(&tw, id).issuer, PrincipalId::Organization(o.org));

    ok(&mut tw, o.member, &format!("@e6/accept {id}"));
    let Ok(AcceptOutcome::NeedsApproval(request)) = e6.accepted() else {
        panic!("expected NeedsApproval, got {:?}", e6.accepted())
    };
    assert!(snap(&tw).approval(request).is_some());
    let c = contract(&tw, id);
    assert!(
        c.acceptances.is_empty(),
        "an unapproved accept recorded an acceptance"
    );
    assert!(c.escrow.is_empty());
    assert_eq!(held(&tw, o.acct), 0);
    assert_eq!(c.state, ContractState::Offered);
}

// ======================================================================
// ---- E6a step 1: S5 — the contract renders (E2-C16) ----
// ======================================================================

/// Two of 08g §3's raw-minor sites: the `@contract/pay` reply and the
/// `@contract/show` line for an `Obligation` deliverable. Both now render in the
/// account's instrument, symbol and scale included.
#[test]
fn e6s1_s5_the_contract_pay_reply_and_the_obligation_deliverable_line_render_at_scale() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    ok(&mut tw, GOD, "@instrument/register yen=2 display=¥:prefix");
    ok(&mut tw, GOD, "@account/system yenfaucet=yen");
    ok(&mut tw, GOD, &format!("@account/open #{}=yen", r.johnson.0));
    ok(&mut tw, GOD, &format!("@account/open #{}=yen", r.a.0));
    let yen_of = |tw: &TestWorld, who: ObjId| -> AccountId {
        snap(tw)
            .accounts()
            .find(|x| x.owner == PrincipalId::Object(who) && x.instrument.as_str() == "yen")
            .expect("yen wallet")
            .id
    };
    let j_yen = yen_of(&tw, r.johnson);
    let a_yen = yen_of(&tw, r.a);
    let faucet = snap(&tw)
        .accounts()
        .find(|x| x.instrument.as_str() == "yen" && matches!(x.owner, PrincipalId::System(_)))
        .expect("yen faucet")
        .id;
    ok(&mut tw, GOD, &format!("@econ/mint {faucet}={j_yen} 100000"));

    ok(&mut tw, r.johnson, "@contract/draft Retainer");
    let id = only_contract(&tw);
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/party {id}=#{}", r.a.0),
    );
    let seen = ok(
        &mut tw,
        r.johnson,
        &format!("@contract/pay {id}={j_yen} 12345"),
    );
    assert!(
        seen.contains("escrows ¥123.45 at acceptance"),
        "the pay reply printed raw minor units: {seen}"
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/deliver {id}=obligation {j_yen} {a_yen} 5000 every=100"),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/split {id}={a_yen} 1000‰"),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/remainder {id}={j_yen}"),
    );
    let seen = ok(&mut tw, r.johnson, &format!("@contract/show {id}"));
    assert!(
        seen.contains("escrows ¥123.45 from account"),
        "the show page printed the consideration raw: {seen}"
    );
    // The creditor's wallet is masked for the Johnson; the amount is not.
    assert!(
        seen.contains("owes an account ¥50.00 each period"),
        "the obligation deliverable printed raw minor units: {seen}"
    );
}

// ======================================================================
// ---- E6a step 1: S6 — a `System` payer is refused structurally (M4, D-E6-4) ----
//
// Nobody acts for a `System` principal, so a contract naming one could be
// offered and never accepted: a zombie holding an offer open forever. The tree
// made it unfundable; this makes it unofferable — decided on the account's owner,
// rendered as the account's own existence-hiding text, with no draft left behind.
// ======================================================================

#[test]
fn e6s1_s6_staff_naming_a_faucet_as_payer_is_refused_with_the_hiding_text_and_nothing_changes() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    let faucet = faucet(&tw);
    ok(&mut tw, GOD, "@contract/draft Faucet");
    let id = only_contract(&tw);
    ok(&mut tw, GOD, &format!("@contract/party {id}=#{}", r.a.0));
    let before = contract(&tw, id);

    // `/pay`: the account's own hiding text — staff may VIEW the faucet, and still get
    // "No such account.", never a faucet-shaped message (M4 ii).
    let seen = err(&mut tw, GOD, &format!("@contract/pay {id}={faucet} 100"));
    assert!(seen.contains("No such account."), "wrong refusal: {seen}");
    assert!(
        !seen.to_ascii_lowercase().contains("system")
            && !seen.to_ascii_lowercase().contains("faucet"),
        "the refusal was faucet-shaped: {seen}"
    );
    // `/party`: a system principal is never a party.
    let seen = err(&mut tw, GOD, &format!("@contract/party {id}=sys:faucet"));
    assert!(seen.contains("No such principal."), "wrong refusal: {seen}");
    // M17: a refused builder changed nothing and created nothing.
    assert_eq!(
        contract(&tw, id),
        before,
        "a refused /pay or /party changed the draft"
    );
    assert_eq!(snap(&tw).contracts().count(), 1);

    // The structural gate `/offer` shares: a `System` party, or a `System` `from`.
    let sys = PrincipalId::System(mush_org::SystemPrincipalId::new("faucet").expect("id"));
    let mut c = contract(&tw, id);
    c.consent.parties.push(sys.clone());
    assert_eq!(
        mush_economy::check_structure(&c, &mush_economy::CanonOutcomeKinds::default()),
        Err(ContractError::SystemParty)
    );
    let mut c = contract(&tw, id);
    c.consent
        .terms
        .consideration
        .push(mush_economy::Consideration {
            from: sys.clone(),
            account: faucet,
            amount: Amount::from_minor(100),
        });
    assert_eq!(
        mush_economy::check_structure(&c, &mush_economy::CanonOutcomeKinds::default()),
        Err(ContractError::SystemParty)
    );
    // ...and boot runs that gate on every stored non-terminal record: a draft that
    // somehow names a faucet is quarantined, never offered.
    let mut kv = fold_kv(&mut tw);
    let patched = serde_json::to_value(&sys).expect("a principal serializes");
    patch(&mut kv, &format!("contract:{id}"), |v| {
        v["consent"]["parties"]
            .as_array_mut()
            .expect("parties")
            .push(patched.clone());
    });
    reboot(&mut tw, kv);
    assert_eq!(state(&tw, id), ContractState::Disputed);
    assert!(
        store(&tw)
            .escalations()
            .any(|e| e.subject == EscalationSubject::Contract(id)),
        "boot quarantined a faucet-party draft without paging"
    );
    let seen = err(&mut tw, GOD, &format!("@contract/offer {id}"));
    assert!(seen.contains("is not a draft"), "{seen}");
}

#[test]
fn e6s1_s6_offer_contract_refuses_a_faucet_payer_or_party_and_leaves_no_draft() {
    let (mut tw, e6) = boot_e6();
    let r = run_rig(&mut tw, 3_000);
    let faucet = faucet(&tw);
    // Staff post with the faucet as the payer's account.
    let spec = run_spec(
        "Faucet",
        GOD,
        (faucet, 100),
        vec![PrincipalId::Object(r.a)],
        vec![row("success", vec![0], vec![exact(r.a_acct, 100)])],
    );
    e6.stage_offer(spec);
    ok(&mut tw, GOD, "@e6/offer");
    assert_eq!(e6.offered(), Err(ContractError::SystemParty));
    assert_eq!(
        snap(&tw).contracts().count(),
        0,
        "a refused offer left a draft"
    );
    // A `System` party is refused just the same, before anything is allocated.
    let spec = run_spec(
        "Faucet party",
        r.johnson,
        (r.j_acct, 100),
        vec![PrincipalId::System(
            mush_org::SystemPrincipalId::new("faucet").expect("id"),
        )],
        vec![row("success", vec![0], vec![exact(r.j_acct, 100)])],
    );
    e6.stage_offer(spec);
    ok(&mut tw, r.johnson, "@e6/offer");
    assert_eq!(e6.offered(), Err(ContractError::SystemParty));
    assert_eq!(snap(&tw).contracts().count(), 0);
    assert_eq!(store(&tw).snapshot().contracts().count(), 0);
}

// ======================================================================
// ---- E6a step 1: S8 — `contract_notice` and the exported hiding helpers ----
// ======================================================================

/// The notice for a run — the class a board entry renders (08f §3.4 `+run/show`) —
/// names the contract, its state, its window and the kinds of condition, and never
/// a payout amount, an account id, a party or a hold.
#[test]
fn e6s1_s8_the_notice_for_a_run_never_carries_an_amount_or_an_account_id() {
    let (mut tw, e6) = boot_e6();
    let r = run_rig(&mut tw, 3_000);
    let fixer_acct = account_of(&tw, r.watcher);
    let mut spec = run_spec(
        "Extraction",
        r.johnson,
        (r.j_acct, 1_000),
        vec![PrincipalId::Object(r.a), PrincipalId::Object(r.b)],
        vec![
            row(
                "success",
                vec![0],
                vec![
                    exact(r.a_acct, 400),
                    exact(r.b_acct, 400),
                    exact(fixer_acct, 200),
                ],
            ),
            row("failure", vec![1], vec![exact(r.j_acct, 1_000)]),
        ],
    );
    spec.deadline = Some(mush_economy::EconomicDeadline {
        clock: mush_economy::EconomicClock::RealUtc,
        at: 5_000_000,
    });
    spec.visibility = EconomicVisibility::Audience {
        members: vec![PrincipalId::Object(r.watcher)],
        announce_issuer: true,
    };
    e6.stage_offer(spec);
    ok(&mut tw, r.johnson, "@e6/offer");
    let id = e6.offered().expect("offered");
    ok(&mut tw, r.johnson, &format!("@e6/accept {id}"));

    let seen = ok(&mut tw, r.watcher, &format!("@e6/notice {id}"));
    assert!(
        seen.contains(&format!("contract #{id} \"Extraction\"")),
        "no name: {seen}"
    );
    assert!(seen.contains("offered"), "no state: {seen}");
    assert!(
        seen.contains("window closes at 5000000"),
        "no window: {seen}"
    );
    assert!(
        seen.contains("conditions: Attestation, Attestation"),
        "no condition kinds: {seen}"
    );
    assert!(
        seen.contains("issued by Johnson"),
        "announce=yes was ignored: {seen}"
    );
    for forbidden in [
        "1000",
        "400",
        "200",
        "account #",
        &format!("account #{}", r.j_acct),
        &format!("account #{}", r.a_acct),
        &format!("account #{}", fixer_acct),
        "Alder",
        "Bex",
        "escrow",
        "hold",
        "payer",
    ] {
        assert!(
            !seen.contains(forbidden),
            "the notice carries {forbidden:?}: {seen}"
        );
    }
    // The same class `@contract/show` renders to an audience member.
    let shown = ok(&mut tw, r.watcher, &format!("@contract/show {id}"));
    assert!(shown.contains("you are seeing a notice"), "{shown}");
    assert!(
        !shown.contains("1000") && !shown.contains("account #"),
        "{shown}"
    );
    // And the exported hiding helper answers for the actor exactly as the command
    // path does: a non-viewer's refusal is the nonexistent-contract text.
    let seen = ok(&mut tw, r.c, &format!("@e6/accept {id}"));
    let _ = seen;
    assert_eq!(e6.accepted(), Err(ContractError::NoStanding));
}

// ======================================================================
// ---- E6a step 1 verify fold ----
//
// The adversarial verify's confirmed findings, folded back in. Three lenses —
// D (disclosure and bounds), R (durability and replay), M (money and consent) —
// and one rule under all of them: a seam that has already moved money may not
// afterwards decide it would rather not have.
// ======================================================================

/// A world whose stale-contract age is an hour, so a page and the draft TTL (both
/// seven days by default) do not land on the same sweep fire.
fn boot_short_stale() -> TestWorld {
    let config = mush_economy::EconomyConfig {
        stale_contract_age_secs: 3_600,
        ..mush_economy::EconomyConfig::default()
    };
    TestWorld::with_modules(vec![
        Box::new(
            mush_org::OrgModule::new()
                .with_capabilities(vocabulary())
                .with_scope_use(vec![Arc::new(mush_economy::EconomyScopeUse)]),
        ),
        Box::new(mush_economy::EconomyModule::with_custody(vec![]).with_config(config)),
    ])
}

/// An offered run with **two payers** — Johnson 300 and Alder 200 — so an amendment
/// by Johnson has somebody else's escrow to release and therefore posts.
fn two_payer_run(tw: &mut TestWorld, r: &Run) -> ContractId {
    let faucet = snap(tw)
        .accounts()
        .find(|x| matches!(x.owner, PrincipalId::System(_)))
        .expect("faucet")
        .id;
    ok(tw, GOD, &format!("@econ/mint {faucet}={} 1000", r.a_acct));
    ok(tw, r.johnson, "@contract/draft Extraction");
    let id = only_contract(tw);
    ok(tw, r.johnson, &format!("@contract/party {id}=#{}", r.a.0));
    // A third party who never accepts, so the contract stays `Offered` and remains
    // amendable once both payers have funded it.
    ok(tw, r.johnson, &format!("@contract/party {id}=#{}", r.b.0));
    ok(
        tw,
        r.johnson,
        &format!("@contract/pay {id}={} 300", r.j_acct),
    );
    // Staff adds Alder's side: a payer may not name a wallet they cannot view.
    ok(tw, GOD, &format!("@contract/pay {id}={} 200", r.a_acct));
    // 450, so the surface stays feasible when M4 edits a consideration down to 250.
    ok(
        tw,
        r.johnson,
        &format!("@contract/split {id}={} 450", r.b_acct),
    );
    ok(
        tw,
        r.johnson,
        &format!("@contract/remainder {id}={}", r.j_acct),
    );
    ok(
        tw,
        r.johnson,
        &format!("@contract/condition {id}=attest party:#{}", r.johnson.0),
    );
    ok(tw, r.johnson, &format!("@contract/offer {id}"));
    ok(tw, r.johnson, &format!("@contract/accept {id}"));
    ok(tw, r.a, &format!("@contract/accept {id}"));
    id
}

/// An org treasury of 1000 under `threshold 200` with a window of 500 per hour, and
/// Quinn holding `economy.account.post` on its compartment. The threshold is
/// deliberately wide: every debit below is about the **window** alone.
fn budgeted_treasury(tw: &mut TestWorld) -> Org {
    tw.set_now(1_000);
    let o = org_rig(tw);
    ok(
        tw,
        GOD,
        &format!(
            "@org/grant #{}=Ironhold economy.account.post scope=#{}",
            o.member.0, o.scope.0
        ),
    );
    ok(
        tw,
        GOD,
        &format!(
            "@account/approval {}=200 threshold:1 window=500/3600",
            o.acct
        ),
    );
    o
}

#[test]
fn vf_m1_an_obligation_period_is_exempt_from_the_window_and_still_counts_against_it() {
    let mut tw = boot();
    let o = budgeted_treasury(&mut tw);
    // A standing debt on the treasury: 150 a period, first due at 1200.
    ok(
        &mut tw,
        GOD,
        &format!(
            "@obligation/create {}={} 150 every=3600 start=200",
            o.acct, o.payee_acct
        ),
    );
    let oid = store(&tw)
        .snapshot()
        .obligations()
        .map(|x| x.id)
        .next()
        .expect("the obligation");
    // Four discretionary debits fill the hour to 400 of 500.
    for t in [1_000, 1_010, 1_020, 1_030] {
        tw.set_now(t);
        let seen = ok(
            &mut tw,
            o.member,
            &format!("@econ/pay {}={} 100", o.acct, o.payee_acct),
        );
        assert!(seen.contains("Paid: transaction"), "at {t}: {seen}");
    }
    assert_eq!(bal(&tw, o.acct), 600);
    // One fire before anything is due, so the next one is not the first sweep after
    // boot (whose gap classifies as downtime and forgives rather than charges).
    sweep_at(&mut tw, 1_040);

    // The period falls due with the window already at 400: 400 + 150 is over it.
    // D-E6-21 as shipped refused the post, suspended the record and paged a human,
    // and the creditor went unpaid — for a debit the debtor consented to when the
    // obligation was created and that the sweep can collect no approval for.
    let ev = sweep_at(&mut tw, 1_250);
    assert_eq!(
        bal(&tw, o.acct),
        450,
        "the obligation's own period did not settle ({:?}): {}",
        snap(&tw)
            .obligation(oid)
            .map(|x| (x.status.clone(), x.periods_settled)),
        texts(&ev)
    );
    let debt = snap(&tw).obligation(oid).expect("obligation").clone();
    assert_eq!(
        debt.status,
        ObligationStatus::Active,
        "a windowed treasury suspended its own standing debt"
    );
    assert_eq!(debt.settled_total, 1);
    assert!(
        !store(&tw)
            .escalations()
            .any(|e| e.subject == EscalationSubject::Obligation(oid)),
        "the window paged a human about a consented debit"
    );

    // ...and the period still COUNTS: the hour has now seen 550, so the next
    // discretionary spend is over the budget. (Without it counting the hour would
    // read 400 and this would post.)
    tw.set_now(1_300);
    let seen = ok(
        &mut tw,
        o.member,
        &format!("@econ/pay {}={} 100", o.acct, o.payee_acct),
    );
    assert!(
        seen.contains("Approval required"),
        "the obligation's debit did not count toward the window: {seen}"
    );
    assert_eq!(bal(&tw, o.acct), 450, "an unapproved debit posted");
}

#[test]
fn vf_r3_a_forward_clock_jump_does_not_erase_the_rolling_window() {
    let mut tw = boot();
    let o = budgeted_treasury(&mut tw);
    tw.set_now(1_000);
    ok(
        &mut tw,
        o.member,
        &format!("@econ/pay {}={} 100", o.acct, o.payee_acct),
    );
    // One post whose wall clock is a decade ahead. Pruning against *that* post's own
    // `posted_at - 90 d` dropped every earlier entry for good, and replay repeated
    // it, so a reboot did not heal it either.
    let jump = 1_000 + 10 * mush_economy::MAX_APPROVAL_WINDOW_SECS;
    tw.set_now(jump);
    ok(
        &mut tw,
        o.member,
        &format!("@econ/pay {}={} 100", o.acct, o.payee_acct),
    );
    let tail = store(&tw)
        .debit_window_of(o.acct)
        .expect("a windowed account keeps a tail");
    assert_eq!(tail.len(), 2, "the jumped post erased the earlier spend");
    // Read back at the earlier clock: the first spend still counts.
    assert_eq!(
        store(&tw).spent_in_window(o.acct, 1_100, 3_600).minor(),
        200
    );

    // Identical after a reboot - the same fold runs over the same journal.
    let kv = fold_kv(&mut tw);
    reboot(&mut tw, kv);
    let tail = store(&tw)
        .debit_window_of(o.acct)
        .expect("the tail did not survive the reboot");
    assert_eq!(tail.len(), 2, "replay repeated the erasure");
    assert_eq!(
        store(&tw).spent_in_window(o.acct, 1_100, 3_600).minor(),
        200
    );
}

#[test]
fn vf_d4_only_a_windowed_account_is_tracked_and_a_new_window_is_rebuilt_from_the_journal() {
    let mut tw = boot();
    tw.set_now(1_000);
    let o = org_rig(&mut tw);
    ok(
        &mut tw,
        GOD,
        &format!(
            "@org/grant #{}=Ironhold economy.account.post scope=#{}",
            o.member.0, o.scope.0
        ),
    );
    // No rule at all: the index tracks nothing, however much the account spends.
    for _ in 0..3 {
        ok(
            &mut tw,
            o.member,
            &format!("@econ/pay {}={} 100", o.acct, o.payee_acct),
        );
    }
    assert!(
        store(&tw).debit_window_of(o.acct).is_none(),
        "a rule-less account kept a rolling-window deque"
    );
    assert!(
        store(&tw).debit_window_of(o.payee_acct).is_none(),
        "a credited account kept a deque"
    );

    // A rule that GAINS a window starts from what the journal already records -
    // otherwise a treasury could be emptied and then given a budget that read it as
    // having spent nothing.
    ok(
        &mut tw,
        GOD,
        &format!(
            "@account/approval {}=200 threshold:1 window=400/3600",
            o.acct
        ),
    );
    assert_eq!(
        store(&tw).spent_in_window(o.acct, 1_000, 3_600).minor(),
        300,
        "the new window did not adopt the journal's spend"
    );
    let seen = ok(
        &mut tw,
        o.member,
        &format!("@econ/pay {}={} 200", o.acct, o.payee_acct),
    );
    assert!(
        seen.contains("Approval required"),
        "a fresh window started the account at zero: {seen}"
    );

    // A rule that LOSES its window stops being tracked at all.
    ok(
        &mut tw,
        GOD,
        &format!("@account/approval {}=window=off", o.acct),
    );
    assert!(
        store(&tw).debit_window_of(o.acct).is_none(),
        "an account whose window was removed kept its deque"
    );
}

#[test]
fn vf_r1_every_amend_refusal_lands_before_the_refund_posts() {
    let (mut tw, e6) = boot_e6();
    let r = run_rig(&mut tw, 3_000);
    let id = offered_run(&mut tw, &r, 300);
    ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
    ok(&mut tw, r.a, &format!("@contract/accept {id}"));
    let before = journal_len(&tw);
    let held_before = held(&tw, r.j_acct);

    // A `System` party: refused by `check_structure_basic`, which used to run four
    // steps AFTER the A' refund had already posted.
    e6.stage_amend(ContractAmendment {
        add_parties: vec![PrincipalId::System(
            mush_org::SystemPrincipalId::new("faucet").expect("id"),
        )],
        reoffer: true,
        ..ContractAmendment::default()
    });
    ok(&mut tw, r.johnson, &format!("@e6/amend {id}"));
    assert_eq!(e6.amended().err(), Some(ContractError::SystemParty));
    assert_eq!(
        state(&tw, id),
        ContractState::Offered,
        "the refusal amended"
    );
    assert_eq!(journal_len(&tw), before, "a refused amendment posted");
    assert_eq!(
        held(&tw, r.j_acct),
        held_before,
        "escrow moved on a refusal"
    );
    assert_eq!(contract(&tw, id).acceptances.len(), 2);

    // A duplicate party: same rule.
    e6.stage_amend(ContractAmendment {
        add_parties: vec![PrincipalId::Object(r.a)],
        reoffer: true,
        ..ContractAmendment::default()
    });
    ok(&mut tw, r.johnson, &format!("@e6/amend {id}"));
    assert!(matches!(
        e6.amended(),
        Err(ContractError::Infeasible("a party is named twice"))
    ));
    assert_eq!(state(&tw, id), ContractState::Offered);
    assert_eq!(journal_len(&tw), before, "a refused amendment posted");
    assert_eq!(held(&tw, r.j_acct), held_before);
}

#[test]
fn vf_r1_a_handler_error_after_a_seam_post_keeps_the_batch() {
    let (mut tw, e6) = boot_e6();
    let r = run_rig(&mut tw, 3_000);
    let id = two_payer_run(&mut tw, &r);
    let mut kv = fold_kv(&mut tw);

    // The ruleset amends through the seam and then fails for a reason of its own.
    // Dispatch truncates a failed handler's deferred queue, so this used to discard
    // the journal chunk, the hold and account projections and the contract doc while
    // RAM kept every effect of them.
    e6.stage_amend(ContractAmendment::default());
    let ev = tw.run(r.johnson, &format!("@e6/amend {id}=fail"));
    assert!(
        has_error(&ev),
        "the probe was supposed to fail: {}",
        texts(&ev)
    );
    assert!(e6.amended().is_ok(), "the amendment itself was refused");
    let ram = contract(&tw, id);
    assert_eq!(ram.state, ContractState::Draft, "RAM did not amend");

    merge(&mut kv, drain(&mut tw));
    reboot(&mut tw, kv);
    let disk = contract(&tw, id);
    assert_eq!(
        disk.state,
        ContractState::Draft,
        "the failed handler threw away a landed post's writes"
    );
    assert_eq!(disk.acceptances.len(), ram.acceptances.len());
    assert_eq!(disk.escrow, ram.escrow);
    assert_eq!(
        held(&tw, r.j_acct),
        300,
        "the author's escrow did not survive the reboot"
    );
    assert_eq!(
        held(&tw, r.a_acct),
        0,
        "the refund the handler's error discarded came back"
    );
}

#[test]
fn vf_r2_a_lost_amend_doc_write_still_reaches_a_terminal_state() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    let id = two_payer_run(&mut tw, &r);
    assert_eq!(held(&tw, r.a_acct), 200);

    let mut kv = fold_kv(&mut tw);
    let pre = doc_of(&kv, &format!("contract:{id}"));
    ok(&mut tw, r.johnson, &format!("@contract/amend {id}"));
    assert_eq!(held(&tw, r.a_acct), 0, "the amendment did not refund Alder");
    assert_eq!(contract(&tw, id).amendments, 1);

    // Lose the companion doc write: the refund landed, the record did not.
    merge(&mut kv, drain(&mut tw));
    patch(&mut kv, &format!("contract:{id}"), |v| *v = pre.clone());
    reboot(&mut tw, kv);
    assert_eq!(state(&tw, id), ContractState::Offered);

    // Re-amend. The voided party's hold is already terminal, so no refund posts -
    // and the generation must move anyway, or `contract/{id}/refund/0` stays a spent
    // key and every later unwind is an `IdempotencyConflict` with the escrow frozen.
    ok(&mut tw, r.johnson, &format!("@contract/amend {id}"));
    assert_eq!(
        contract(&tw, id).amendments,
        1,
        "the retry left the generation on a spent key"
    );
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    ok(&mut tw, GOD, &format!("@contract/cancel {id}"));
    assert_eq!(state(&tw, id), ContractState::Cancelled);
    assert_eq!(held(&tw, r.j_acct), 0, "the escrow stayed frozen");
    assert_eq!(bal(&tw, r.j_acct), 3_000);
    assert!(
        !store(&tw)
            .escalations()
            .any(|e| e.context.to_lowercase().contains("idempotency")),
        "an idempotency incident was paged"
    );
}

#[test]
fn vf_m2_an_escrowed_deadline_less_draft_is_paged_and_then_unwound() {
    let mut tw = boot_short_stale();
    tw.set_now(1_000);
    let r = run_rig(&mut tw, 3_000);
    let id = offered_run(&mut tw, &r, 300);
    ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
    ok(&mut tw, r.johnson, &format!("@contract/amend {id}"));
    assert_eq!(state(&tw, id), ContractState::Draft);
    assert_eq!(held(&tw, r.j_acct), 300, "the author's escrow left");
    assert!(contract(&tw, id).consent.deadline.is_none());

    // The stale age pages first: an amendment nobody re-offered is money reserved
    // with nobody looking at it.
    sweep_at(&mut tw, 1_000 + 3_600 + 600);
    let paged = store(&tw)
        .escalations()
        .find(|e| e.subject == EscalationSubject::Contract(id))
        .cloned()
        .expect("an escrowed draft was never paged");
    assert!(
        paged.context.contains("draft"),
        "the page does not say what it is about: {}",
        paged.context
    );
    assert!(
        paged
            .recommended
            .iter()
            .any(|c| c.contains("@contract/offer")),
        "the page recommends nothing that works: {:?}",
        paged.recommended
    );
    assert_eq!(state(&tw, id), ContractState::Draft, "the page unwound it");

    // ...and the draft TTL is the backstop: with no deadline, nothing else would
    // ever have touched it.
    sweep_at(&mut tw, 1_000 + mush_economy::DRAFT_TTL_SECS + 600);
    assert_eq!(state(&tw, id), ContractState::Expired);
    assert_eq!(held(&tw, r.j_acct), 0, "the escrow was never released");
    assert_eq!(bal(&tw, r.j_acct), 3_000);
}

#[test]
fn vf_m4_an_escrow_hold_that_claims_no_consideration_survives_an_amendment() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    let id = two_payer_run(&mut tw, &r);
    let johnsons = contract(&tw, id).escrow[0];
    let alders = contract(&tw, id).escrow[1];

    // A `Held` hold, purpose `Escrow { this contract }`, that **claims no
    // consideration**: `build_pay`'s guards make it unreachable live, so it is
    // reached the way a migration or a hand edit reaches it — the stored surface is
    // edited (the payer's consideration is 250, the hold it placed is 300) and
    // re-hashed, so the record is internally consistent and only the claim is gone.
    // The old rule built `escrow` from the author's *claims*, so this hold was
    // neither released nor kept: money nothing would ever have let go of again.
    let mut kv = fold_kv(&mut tw);
    let key = format!("contract:{id}");
    let doc = doc_of(&kv, &key);
    let mut edited: mush_economy::EconomicContract =
        serde_json::from_value(doc).expect("the contract doc parses");
    edited.consent.terms.consideration[0].amount = Amount::from_minor(250);
    edited.consent_hash = mush_economy::consent_hash(&edited.issuer, &edited.consent);
    for a in &mut edited.acceptances {
        a.consent_hash = edited.consent_hash;
    }
    let rewritten = serde_json::to_value(&edited).expect("the contract serializes");
    patch(&mut kv, &key, |v| *v = rewritten.clone());
    reboot(&mut tw, kv.clone());
    let c = contract(&tw, id);
    assert!(c.consent_intact(), "the rewritten surface does not hash");
    assert_eq!(
        mush_economy::qualifying_escrow(&c, snap(&tw))
            .holds
            .iter()
            .map(|h| h.id)
            .collect::<Vec<_>>(),
        vec![alders],
        "the payer's hold should now claim nothing"
    );

    ok(&mut tw, r.johnson, &format!("@contract/amend {id}"));
    let c = contract(&tw, id);
    assert!(
        c.escrow.contains(&johnsons),
        "the amendment orphaned an escrow hold it did not release: {:?}",
        c.escrow
    );
    assert!(
        !c.escrow.contains(&alders),
        "the refunded hold stayed on the record: {:?}",
        c.escrow
    );
    assert!(
        snap(&tw).hold(johnsons).is_some_and(|h| h.status.is_held()),
        "the orphan was released"
    );
    assert_eq!(held(&tw, r.j_acct), 300, "the author's escrow moved");
    // A reboot finds it exactly where the record says it is — and boot's orphan pass
    // has nothing to adopt, because the record never let go of it.
    merge(&mut kv, drain(&mut tw));
    reboot(&mut tw, kv);
    assert!(contract(&tw, id).escrow.contains(&johnsons));
    assert!(snap(&tw).hold(johnsons).is_some_and(|h| h.status.is_held()));
}

#[test]
fn vf_m5_an_author_only_reoffer_activates_instead_of_stranding() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    // A one-party contract: the author is the only party, so their own accept
    // activates it (D-E4-10) - a run board's `Listed` entry.
    ok(&mut tw, r.johnson, "@contract/draft Listing");
    let id = only_contract(&tw);
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/pay {id}={} 300", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/split {id}={} 300", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/remainder {id}={}", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/condition {id}=attest party:#{}", r.johnson.0),
    );
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
    assert_eq!(state(&tw, id), ContractState::Active);

    // A': amend the live, author-only contract and re-offer it. The author's
    // acceptance survives, so the re-offer is already unanimous - and nothing else
    // in the world promotes an `Offered` contract that is `fully_accepted()`.
    ok(&mut tw, r.johnson, &format!("@contract/amend {id}"));
    assert_eq!(state(&tw, id), ContractState::Draft);
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    assert_eq!(
        state(&tw, id),
        ContractState::Active,
        "an already-unanimous re-offer stranded in Offered"
    );
    assert_eq!(held(&tw, r.j_acct), 300, "the escrow moved");
}

#[test]
fn vf_d2_an_unviewable_account_renders_exactly_like_an_unknown_one() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    let s = snap(&tw);
    let amount = Amount::from_minor(1_250);
    // A known account the caller may not view, and an account that does not exist:
    // the render used to give the first its scale and symbol and the second a bare
    // minor value, which is an existence oracle over any id.
    assert_eq!(
        mush_economy::render_amount_on(s, r.j_acct, amount, false, None),
        mush_economy::render_amount_on(s, AccountId(4_242), amount, true, None),
        "an unviewable account renders differently from an unknown one"
    );
    // The gate is the only difference: the same account, viewable, renders in its
    // own instrument.
    let instrument = s.account(r.j_acct).expect("account").instrument.clone();
    assert_eq!(
        mush_economy::render_amount_on(s, r.j_acct, amount, true, None),
        mush_economy::render_amount(s, &instrument, amount)
    );
}

#[test]
fn vf_m8_an_unviewable_payout_still_renders_in_the_contracts_instrument() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    let id = offered_run(&mut tw, &r, 300);
    // Alder is a party (so may read the contract) and has no standing at all over
    // Johnson's wallet.
    assert!(!mush_economy::may_view_account(tw.world(), r.a, r.j_acct));
    let seen = ok(&mut tw, r.a, &format!("@contract/show {id}"));
    assert!(
        seen.contains("escrows 300 from an account"),
        "the masked consideration lost its instrument: {seen}"
    );
    // And a payout row whose account no longer resolves at all reads the same way,
    // rather than as a raw minor value at scale 0.
    let mut kv = fold_kv(&mut tw);
    patch(&mut kv, &format!("contract:{id}"), |v| {
        v["consent"]["terms"]["split"] =
            serde_json::json!([{ "to": 4242, "share": { "Exact": 300 } }]);
    });
    reboot(&mut tw, kv);
    let seen = ok(&mut tw, r.a, &format!("@contract/show {id}"));
    assert!(
        seen.contains("pays an account 300"),
        "an unresolvable payee lost the contract's instrument: {seen}"
    );
}

#[test]
fn vf_r7_a_draft_whose_escrow_is_gone_is_collected_on_one_predicate() {
    let mut tw = boot();
    tw.set_now(1_000);
    let r = run_rig(&mut tw, 3_000);
    ok(&mut tw, r.johnson, "@contract/draft Abandoned");
    let id = only_contract(&tw);
    // The record lists an escrow id that is not real escrow for it — `c.escrow` is an
    // ordinary mutable field, which is exactly why `qualifying_escrow` exists. The
    // sweep selected this draft for collection on `qualifying_escrow` (empty) and
    // `collect_draft` refused it on `c.escrow` (non-empty), so it burned a per-fire
    // slot every fire, forever, and was never collected.
    let mut kv = fold_kv(&mut tw);
    patch(&mut kv, &format!("contract:{id}"), |v| {
        v["escrow"] = serde_json::json!([9_999]);
    });
    reboot(&mut tw, kv);
    assert!(!contract(&tw, id).escrow.is_empty());
    assert!(
        mush_economy::qualifying_escrow(&contract(&tw, id), snap(&tw))
            .holds
            .is_empty(),
        "the listed id was real escrow after all"
    );

    sweep_at(&mut tw, 1_000 + mush_economy::DRAFT_TTL_SECS + 600);
    assert!(
        snap(&tw).contract(id).is_none(),
        "the abandoned draft was never collected"
    );
}

#[test]
fn vf_m6_a_faucet_party_quarantine_recommends_only_a_refund() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    let id = offered_run(&mut tw, &r, 300);
    let sys = PrincipalId::System(mush_org::SystemPrincipalId::new("faucet").expect("id"));
    let mut kv = fold_kv(&mut tw);
    let patched = serde_json::to_value(&sys).expect("a principal serializes");
    patch(&mut kv, &format!("contract:{id}"), |v| {
        v["consent"]["parties"]
            .as_array_mut()
            .expect("parties")
            .push(patched.clone());
    });
    reboot(&mut tw, kv);
    let paged = store(&tw)
        .escalations()
        .find(|e| e.subject == EscalationSubject::Contract(id))
        .cloned()
        .expect("boot quarantined without paging");
    // `settlement_plan` re-runs the very check that quarantined this contract, so
    // recommending `=settle` was recommending a refusal.
    assert!(
        !paged.recommended.iter().any(|c| c.contains("=settle")),
        "the page recommends a command that cannot work: {:?}",
        paged.recommended
    );
    assert!(
        paged.recommended.iter().any(|c| c.contains("=refund")),
        "the page recommends nothing that does: {:?}",
        paged.recommended
    );
}

#[test]
fn vf_d10_an_over_cap_notice_document_is_trimmed_before_it_is_parsed() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    let mut kv = fold_kv(&mut tw);
    // Ten thousand entries used to be parsed one by one - byte cap and all - and
    // then thrown away down to thirty-two.
    let bloat: Vec<serde_json::Value> = (0..10_000)
        .map(|i| serde_json::json!({ "at": 1, "text": format!("line {i}") }))
        .collect();
    kv.push((
        mush_economy::notice_key(r.johnson),
        serde_json::Value::Array(bloat),
    ));
    let started = std::time::Instant::now();
    reboot(&mut tw, kv);
    let loaded = store(&tw).pending_notices(r.johnson);
    assert_eq!(loaded.len(), mush_economy::MAX_PENDING_NOTICES);
    assert_eq!(
        loaded.last().map(|n| n.text.to_string()),
        Some("line 9999".to_string()),
        "the newest entries were not the ones kept"
    );
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "the trimmed load was not fast"
    );
}

#[test]
fn vf_r9_the_notice_queue_path_carries_no_panicking_call() {
    // R9: `notice.rs` runs on observer and tick paths - an `expect` there aborts the
    // world task mid-sweep. A source scan, like `provider_rng_policy.rs`, because the
    // failure it guards against is one no ordinary run reaches.
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/notice.rs"))
        .expect("the notice module is readable");
    for (n, line) in src.lines().enumerate() {
        let code = line.split("//").next().unwrap_or(line);
        for bad in [".expect(", ".unwrap()", "panic!("] {
            assert!(
                !code.contains(bad),
                "notice.rs:{} carries {bad} on an observer/tick path: {line}",
                n + 1
            );
        }
    }
}

#[test]
fn vf_d8_the_account_help_names_the_rolling_window() {
    let mut tw = boot();
    let seen = ok(&mut tw, GOD, "help @account");
    assert!(
        seen.contains("window="),
        "the help never mentions the budget it now enforces: {seen}"
    );
    assert!(
        seen.to_lowercase().contains("budget"),
        "the help still says a threshold is the only control: {seen}"
    );
}

// ---- E6a: the cancel seam (08f §3.2's `+run/withdraw` and M17's refused accept) ---

/// The fourth seam: a ruleset cancels the contract it issued without dispatching
/// `@contract/cancel`, and the escrow comes back.
#[test]
fn e6a_cancel_contract_refunds_an_offered_contract_through_the_seam() {
    let (mut tw, e6) = boot_e6();
    let r = run_rig(&mut tw, 3_000);
    let spec = run_spec(
        "Withdrawn",
        r.johnson,
        (r.j_acct, 1_000),
        vec![PrincipalId::Object(r.a)],
        vec![
            row("success", vec![0], vec![exact(r.a_acct, 1_000)]),
            row("failure", vec![1], vec![exact(r.j_acct, 1_000)]),
        ],
    );
    e6.stage_offer(spec);
    ok(&mut tw, r.johnson, "@e6/offer");
    let id = e6.offered().expect("the offer stands");
    ok(&mut tw, r.johnson, &format!("@e6/accept {id}"));
    assert_eq!(held(&tw, r.j_acct), 1_000);
    assert_eq!(state(&tw, id), ContractState::Offered);
    ok(&mut tw, r.johnson, &format!("@e6/cancel {id}"));
    assert_eq!(
        e6.cancelled(),
        Ok(true),
        "the cancel did not reach terminal"
    );
    assert_eq!(state(&tw, id), ContractState::Cancelled);
    assert_eq!(held(&tw, r.j_acct), 0, "the escrow was not released");
    assert_eq!(bal(&tw, r.j_acct), 3_000);
}

/// A′'s other face: a **live** contract with no deliverables that nobody but its own
/// author has accepted is cancellable — that is exactly a run board's `Listed`
/// entry, and "a live agreement is not one party's to tear up" is a rule about other
/// people's consent.
#[test]
fn e6a_cancel_contract_admits_an_author_only_live_contract() {
    let (mut tw, e6) = boot_e6();
    let r = run_rig(&mut tw, 3_000);
    let spec = run_spec(
        "Listed",
        r.johnson,
        (r.j_acct, 1_000),
        Vec::new(),
        vec![
            row("success", vec![0], vec![exact(r.j_acct, 1_000)]),
            row("failure", vec![1], vec![exact(r.j_acct, 1_000)]),
        ],
    );
    e6.stage_offer(spec);
    ok(&mut tw, r.johnson, "@e6/offer");
    let id = e6.offered().expect("the offer stands");
    ok(&mut tw, r.johnson, &format!("@e6/accept {id}"));
    assert_eq!(
        state(&tw, id),
        ContractState::Active,
        "a one-party contract did not activate at its author's accept"
    );
    ok(&mut tw, r.johnson, &format!("@e6/cancel {id}"));
    assert_eq!(e6.cancelled(), Ok(true));
    assert_eq!(state(&tw, id), ContractState::Cancelled);
    assert_eq!(held(&tw, r.j_acct), 0);
    assert_eq!(bal(&tw, r.j_acct), 3_000);
}

/// …and a contract somebody **else** accepted is not: the routes stay the dispute,
/// the deadline and staff. Nothing moves on the refusal.
#[test]
fn e6a_cancel_contract_refuses_a_contract_another_party_accepted() {
    let (mut tw, e6) = boot_e6();
    let r = run_rig(&mut tw, 3_000);
    ok(&mut tw, GOD, &format!("@econ/mint 0={} 2000", r.a_acct));
    let spec = run_spec(
        "Bound",
        r.johnson,
        (r.j_acct, 1_000),
        vec![PrincipalId::Object(r.a)],
        vec![
            row("success", vec![0], vec![exact(r.a_acct, 1_000)]),
            row("failure", vec![1], vec![exact(r.j_acct, 1_000)]),
        ],
    );
    e6.stage_offer(spec);
    ok(&mut tw, r.johnson, "@e6/offer");
    let id = e6.offered().expect("the offer stands");
    ok(&mut tw, r.johnson, &format!("@e6/accept {id}"));
    ok(&mut tw, r.a, &format!("@e6/accept {id}"));
    assert_eq!(state(&tw, id), ContractState::Active);
    let before = journal_len(&tw);
    ok(&mut tw, r.johnson, &format!("@e6/cancel {id}"));
    assert!(
        matches!(e6.cancelled(), Err(ContractError::State(_))),
        "a bound contract was cancellable: {:?}",
        e6.cancelled()
    );
    assert_eq!(state(&tw, id), ContractState::Active);
    assert_eq!(held(&tw, r.j_acct), 1_000, "escrow moved on a refusal");
    assert_eq!(journal_len(&tw), before, "a refusal posted");
}

/// A non-issuer gets the one answer every standing failure gets — "No such
/// contract." — and nothing changes.
#[test]
fn e6a_cancel_contract_answers_a_non_issuer_with_no_standing() {
    let (mut tw, e6) = boot_e6();
    let r = run_rig(&mut tw, 3_000);
    let spec = run_spec(
        "Theirs",
        r.johnson,
        (r.j_acct, 1_000),
        vec![PrincipalId::Object(r.a)],
        vec![
            row("success", vec![0], vec![exact(r.a_acct, 1_000)]),
            row("failure", vec![1], vec![exact(r.j_acct, 1_000)]),
        ],
    );
    e6.stage_offer(spec);
    ok(&mut tw, r.johnson, "@e6/offer");
    let id = e6.offered().expect("the offer stands");
    ok(&mut tw, r.johnson, &format!("@e6/accept {id}"));
    ok(&mut tw, r.a, &format!("@e6/cancel {id}"));
    assert_eq!(e6.cancelled(), Err(ContractError::NoStanding));
    assert_eq!(state(&tw, id), ContractState::Offered);
    assert_eq!(held(&tw, r.j_acct), 1_000);
}

// ---- E6a verify fold ----

/// D2: an outcome row's **name** is a stored, broadcast string, and the economy caps
/// it for every producer.
///
/// `check_structure` counted the rows and never measured them, so a seam caller — or a
/// hand-written document at boot — could freeze a 4 kB row name into the consent
/// surface, where it is copied into every projection over the contract and rendered by
/// every notice. `@contract/outcome` capped its own input; the programmatic seam did
/// not, which is exactly the shape a ruleset uses.
#[test]
fn vf_d2_the_economy_refuses_an_oversized_outcome_row_from_any_producer() {
    let (mut tw, e6) = boot_e6();
    let r = run_rig(&mut tw, 3_000);
    let long = "x".repeat(mush_economy::MAX_OUTCOME_NAME_BYTES + 1);
    let spec = run_spec(
        "Wordy",
        r.johnson,
        (r.j_acct, 1_000),
        Vec::new(),
        vec![
            row(&long, vec![0], vec![exact(r.j_acct, 1_000)]),
            row("failure", vec![1], vec![exact(r.j_acct, 1_000)]),
        ],
    );
    e6.stage_offer(spec);
    ok(&mut tw, r.johnson, "@e6/offer");
    assert!(
        matches!(e6.offered(), Err(ContractError::TooLarge(_))),
        "an oversized row name was accepted through the seam: {:?}",
        e6.offered()
    );
    assert!(
        snap(&tw).contracts().next().is_none(),
        "a contract was written"
    );

    // The bound is a bound and not a ban: exactly the cap is fine.
    let at_cap = "y".repeat(mush_economy::MAX_OUTCOME_NAME_BYTES);
    let spec = run_spec(
        "Wordy",
        r.johnson,
        (r.j_acct, 1_000),
        Vec::new(),
        vec![
            row(&at_cap, vec![0], vec![exact(r.j_acct, 1_000)]),
            row("failure", vec![1], vec![exact(r.j_acct, 1_000)]),
        ],
    );
    e6.stage_offer(spec);
    ok(&mut tw, r.johnson, "@e6/offer");
    assert!(e6.offered().is_ok(), "the cap itself was refused");
}

// ---- E6b: B3 — DL-03, an org or system depositor's deposit comes home -----

/// The pure shape of DL-03, over a hand-built snapshot: an **organization**
/// depositor with an `Active` vault of its own is routed home to that vault's
/// backing, and `refund_plan` emits the `Withdraw` that takes it there.
///
/// Before E6b this depositor was filtered out of `returns` entirely — "no world
/// object to hand the thing back to" — and its goods stayed in the counterparty's
/// vault with the lock gone and nothing naming them.
#[test]
fn e6b_b3_an_org_depositors_deposit_is_routed_home_to_its_own_vault() {
    let (mut s, c) = dl03_fixture(true);
    let returns = mush_economy::deposit_returns(&c, &s);
    assert_eq!(returns.len(), 1);
    assert_eq!(returns[0].to, Some(mush_core::ObjId(77)));
    assert_eq!(returns[0].into_vault, Some(VaultId(9)));

    let plan = mush_economy::refund_plan(&c, &s).expect("the refund builds");
    assert_eq!(
        plan.custody_ops,
        vec![mush_economy::CustodyOp::Withdraw {
            object: mush_core::ObjId(50),
            vault: VaultId(1),
            to: mush_core::ObjId(77),
        }],
        "the deposit was not routed home"
    );
    // The authority agrees with the plan, and only for that exact route.
    let auth = ContractAuth::unwinding(&c, &s);
    let vault = s.vault(VaultId(1)).expect("the counterparty vault").clone();
    use mush_economy::AuthEval;
    assert!(auth.authorize_vault(
        &PrincipalId::Organization(OrgId(3)),
        &vault,
        &mush_economy::CustodyOp::Withdraw {
            object: mush_core::ObjId(50),
            vault: VaultId(1),
            to: mush_core::ObjId(77),
        }
    ));
    assert!(auth.authorize_vault_home(mush_core::ObjId(50), mush_core::ObjId(77)));
    // …and never for anywhere else, nor for any other object.
    assert!(!auth.authorize_vault_home(mush_core::ObjId(50), mush_core::ObjId(78)));
    assert!(!auth.authorize_vault_home(mush_core::ObjId(51), mush_core::ObjId(77)));
    // A **settlement** authority never opens it at all: the exception is
    // authority-shaped, and only an unwind is that authority.
    let settling = ContractAuth::new(&c, &s);
    assert!(!settling.authorize_vault_home(mush_core::ObjId(50), mush_core::ObjId(77)));
    // And `WorldAuth`'s default answer — the one every command reaches — is `false`,
    // so no `@vault/withdraw` can ever name a vault backing as a destination.
    s.put_vault(vault);
}

/// A depositor organization with **no vault** is `to: None` — the stranded shape the
/// unwind pages on rather than silently dropping (08a's staff-scarce rule).
#[test]
fn e6b_b3_a_depositor_org_with_no_vault_is_stranded_not_silently_dropped() {
    let (s, c) = dl03_fixture(false);
    let returns = mush_economy::deposit_returns(&c, &s);
    assert_eq!(returns.len(), 1);
    assert_eq!(returns[0].to, None, "a vault appeared from nowhere");
    assert!(mush_economy::owner_home(&s, &PrincipalId::Organization(OrgId(3))).is_none());
    // The refund still builds and still releases the escrow: one stranded deposit
    // never blocks giving everybody else their money back.
    let plan = mush_economy::refund_plan(&c, &s).expect("the refund still builds");
    assert!(plan.custody_ops.is_empty());
}

/// A cancelled contract with one recorded deposit from `org:3` into vault 1, and
/// (optionally) a vault of its own to come home to.
fn dl03_fixture(org_has_vault: bool) -> (EconomySnapshot, mush_economy::EconomicContract) {
    use mush_economy::{
        ConsentSurface, ContractRoles, ContractTerms, CustodyRef, DepositRecord, EconomicContract,
        EconomicVisibility, Expiry, InstrumentDef, InstrumentKind, InstrumentStatus,
        NegativePolicy, Vault, VaultStatus,
    };
    let mut s = EconomySnapshot::default();
    let instrument = mush_economy::InstrumentId::new("gold").expect("well-formed");
    s.put_instrument(InstrumentDef {
        id: instrument.clone(),
        ruleset: SmolStr::new_static("test"),
        code: SmolStr::new_static("gold"),
        kind: InstrumentKind::Currency,
        scale: 0,
        transferable: true,
        negative_policy: NegativePolicy::SystemOnly,
        status: InstrumentStatus::Active,
        display: None,
        revision: 1,
    });
    // The counterparty's vault, holding the org's deposit.
    s.put_vault(Vault {
        id: VaultId(1),
        owner: PrincipalId::Object(mush_core::ObjId(5)),
        scope: None,
        name: SmolStr::new_static("fence box"),
        backing: CustodyRef::Container(mush_core::ObjId(60)),
        status: VaultStatus::Active,
        revision: 1,
    });
    if org_has_vault {
        s.put_vault(Vault {
            id: VaultId(9),
            owner: PrincipalId::Organization(OrgId(3)),
            scope: None,
            name: SmolStr::new_static("vendor store"),
            backing: CustodyRef::Container(mush_core::ObjId(77)),
            status: VaultStatus::Active,
            revision: 1,
        });
    }
    let c = EconomicContract {
        id: ContractId(1),
        name: SmolStr::new_static("consignment"),
        issuer: PrincipalId::Object(mush_core::ObjId(5)),
        consent: ConsentSurface {
            parties: vec![
                PrincipalId::Object(mush_core::ObjId(5)),
                PrincipalId::Organization(OrgId(3)),
            ],
            terms: ContractTerms {
                consideration: Vec::new(),
                deliverables: Vec::new(),
                split: Vec::new(),
                remainder_to: AccountId(0),
                on_expiry: Expiry::Refund,
                outcomes: Vec::new(),
            },
            conditions: Vec::new(),
            deadline: None,
            visibility: EconomicVisibility::Parties,
            scope: None,
            adjudicator: None,
            evidence: Vec::new(),
        },
        acceptances: Vec::new(),
        satisfied: Vec::new(),
        escrow: Vec::new(),
        deposits: vec![DepositRecord {
            object: mush_core::ObjId(50),
            vault: VaultId(1),
            deposited_by: PrincipalId::Organization(OrgId(3)),
            at: 10,
        }],
        state: ContractState::Cancelled,
        roles: ContractRoles::default(),
        consent_hash: [0u8; 32],
        amendments: 0,
        generation_started_at: 0,
        roster_locked: false,
        pending: None,
        canon_published: None,
        created_at: 1,
        settle_attempts: 0,
        disputed_at: None,
        agreements: Vec::new(),
        revision: 1,
    };
    let mut c = c;
    c.consent_hash = mush_economy::consent_hash(&c.issuer, &c.consent);
    (s, c)
}

// ---- E6b verify fold — money and consent ---------------------------------

/// **M1.** A `DepositInto` condition may only name a vault whose **owner is a
/// party**. E6b let that condition stand in for the vault owner's consent
/// (`WorldAuth::authorize_vault`), and it was not consent at all: anyone holding
/// `vault.view` on a compartment could name that vault in a one-party contract,
/// deposit into it, and leave the object **custody-locked** where its owner could
/// not take it out — a permanent wedge, repeatable until the vault was full.
#[test]
fn vf_m1_a_deposit_condition_names_a_vault_its_owner_is_a_party_to() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 1_000);
    // A vault the intruder does not own and is not in a contract with.
    let backing = tw
        .world_mut()
        .create(ObjKind::Thing, "strongbox", GOD, None)
        .expect("a container");
    ok(
        &mut tw,
        GOD,
        &format!(
            "@vault/create Box=container:#{} owner=#{}",
            backing.0, r.b.0
        ),
    );
    let vault = snap(&tw).vaults().map(|v| v.id).max().expect("a vault");
    let goods = tw
        .world_mut()
        .create(ObjKind::Thing, "crowbar", GOD, Some(r.a))
        .expect("an object");

    // Staff draft it, so `may_view_vault` is not what refuses — the party rule is.
    ok(&mut tw, GOD, "@contract/draft Wedge");
    let id = snap(&tw)
        .contracts()
        .map(|c| c.id)
        .max()
        .expect("a contract");
    ok(&mut tw, GOD, &format!("@contract/pay {id}={} 1", r.a_acct));
    ok(
        &mut tw,
        GOD,
        &format!("@contract/split {id}={} 1", r.a_acct),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@contract/remainder {id}={}", r.a_acct),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@contract/condition {id}=deposit #{} {vault}", goods.0),
    );
    // `r.b` owns the vault and is no party to this, so the offer is refused before
    // an acceptance could ever create the lock.
    err(&mut tw, GOD, &format!("@contract/offer {id}"));
    assert!(
        snapshot_lock_free(&tw, goods, vault),
        "the wedge was created anyway"
    );
}

fn snapshot_lock_free(tw: &TestWorld, object: ObjId, vault: VaultId) -> bool {
    snap(tw).custody_lock(object, vault).is_none()
}

/// **M3.** A theft report is evidence, never a way out of a deal you already agreed
/// to: with `Encumber` staff-only before E6b, no party could put a claim on an asset,
/// and relaxing it for the title owner handed them a settlement veto — accept the
/// escrow, file a *covert* report, and the `TransferTitle` refuses with `Encumbered`
/// while the buyer cannot even see why.
#[test]
fn vf_m3_a_live_deals_asset_cannot_be_reported_stolen_out_from_under_it() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 5_000);
    let obj = tw
        .world_mut()
        .create(ObjKind::Thing, "predator", GOD, Some(r.a))
        .expect("an object");
    ok(&mut tw, GOD, &format!("@asset/title #{}=#{}", obj.0, r.a.0));
    let faucet = snap(&tw)
        .accounts()
        .find(|x| matches!(x.owner, PrincipalId::System(_)))
        .expect("faucet")
        .id;
    ok(
        &mut tw,
        GOD,
        &format!("@econ/mint {faucet}={} 1000", r.a_acct),
    );
    ok(&mut tw, r.a, "@contract/draft Sale");
    let id = snap(&tw)
        .contracts()
        .map(|c| c.id)
        .max()
        .expect("a contract");
    ok(&mut tw, r.a, &format!("@contract/party {id}=#{}", r.b.0));
    ok(
        &mut tw,
        r.a,
        &format!("@contract/pay {id}={} 100", r.a_acct),
    );
    ok(
        &mut tw,
        r.a,
        &format!("@contract/split {id}={} 100", r.b_acct),
    );
    ok(
        &mut tw,
        r.a,
        &format!("@contract/remainder {id}={}", r.a_acct),
    );
    ok(
        &mut tw,
        r.a,
        &format!(
            "@contract/deliver {id}=title #{} #{}->#{}",
            obj.0, r.a.0, r.b.0
        ),
    );
    ok(&mut tw, r.a, &format!("@contract/offer {id}"));
    ok(&mut tw, r.a, &format!("@contract/accept {id}"));
    ok(&mut tw, r.b, &format!("@contract/accept {id}"));

    // The seller cannot now encumber the very thing they sold.
    err(
        &mut tw,
        r.a,
        &format!("@asset/stolen #{}=covert changed my mind", obj.0),
    );
    assert!(
        snap(&tw).asset(obj).expect("the asset").stolen.is_none(),
        "a party vetoed their own settlement with a theft report"
    );
}

/// **R1.** The report goes with its claim. `record.rs` said clearing a theft report
/// was `@asset/release` of its `Disputed` claim, and it was not: nothing anywhere
/// wrote `stolen = None`, so a recovered asset carried the report for ever and
/// refused a **genuine second theft** with `AlreadyClaimed`.
#[test]
fn vf_r1_releasing_the_claim_clears_the_report_so_a_second_theft_can_be_filed() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 1_000);
    let obj = tw
        .world_mut()
        .create(ObjKind::Thing, "predator", GOD, Some(r.a))
        .expect("an object");
    ok(&mut tw, GOD, &format!("@asset/title #{}=#{}", obj.0, r.a.0));
    ok(
        &mut tw,
        r.a,
        &format!("@asset/stolen #{}=public lifted", obj.0),
    );
    assert!(snap(&tw).asset(obj).expect("the asset").stolen.is_some());
    // A second filing is refused while the first stands.
    err(
        &mut tw,
        r.a,
        &format!("@asset/stolen #{}=public again", obj.0),
    );

    ok(&mut tw, GOD, &format!("@asset/release #{}=1", obj.0));
    let asset = snap(&tw).asset(obj).expect("the asset");
    assert!(asset.encumbrances.is_empty());
    assert!(
        asset.stolen.is_none(),
        "the claim was released and the report outlived it"
    );
    // …and the same object can be reported stolen again, which is the point.
    ok(
        &mut tw,
        r.a,
        &format!("@asset/stolen #{}=public taken again", obj.0),
    );
}

// ===========================================================================
// E6b independent verify fold
// ===========================================================================

/// **D2.** `@asset/release <obj>=<n>` resolved the index with **no view gate at
/// all**: out of range answered "No such encumbrance.", in range answered
/// "unauthorized". An assignee holds `may_view_asset`, so both were visible — and the
/// difference between them was a covert theft report, read off an index probe. The
/// hidden claim now answers exactly as an index past the end does.
#[test]
fn vf2_d2_the_release_index_is_not_an_oracle_for_a_covert_report() {
    let mut tw = boot();
    let owner = tw.spawn_player("Owner");
    let assignee = tw.spawn_player("Assignee");
    ok(&mut tw, GOD, "@instrument/register nuyen=0");
    let clean = tw
        .world_mut()
        .create(ObjKind::Thing, "clean case", GOD, Some(owner))
        .expect("an object");
    let claimed = tw
        .world_mut()
        .create(ObjKind::Thing, "hot case", GOD, Some(owner))
        .expect("an object");
    for obj in [clean, claimed] {
        ok(
            &mut tw,
            GOD,
            &format!("@asset/title #{}=#{}", obj.0, owner.0),
        );
        ok(
            &mut tw,
            GOD,
            &format!("@asset/assign #{}=#{} carry", obj.0, assignee.0),
        );
    }
    // A covert report on one of them, filed by its own title owner.
    ok(
        &mut tw,
        owner,
        &format!("@asset/stolen #{}=covert lifted", claimed.0),
    );
    assert!(
        mush_economy::may_view_asset(tw.world(), assignee, claimed),
        "the probe needs an assignee who can see the asset at all"
    );

    // The two worlds differ only in the hidden claim, and the answers are identical.
    let a = err(&mut tw, assignee, &format!("@asset/release #{}=1", clean.0));
    let b = err(
        &mut tw,
        assignee,
        &format!("@asset/release #{}=1", claimed.0),
    );
    assert_eq!(a, b, "the index published the covert claim");
    assert!(a.contains("No such encumbrance."), "{a}");
    // …and the claim is still there for the people who may see it.
    assert_eq!(
        snap(&tw)
            .asset(claimed)
            .map(|x| x.encumbrances.len())
            .unwrap_or_default(),
        1
    );
}

/// **D6.** A `Canon` attestor set's `kind` and `source` are stored strings frozen
/// into the consent surface and rendered to every party, and **nothing bounded
/// either**. `parse_route_spec` takes the source as the remainder verbatim, so any
/// producer of a `ContractSpec` could freeze an arbitrarily large one. The cap is the
/// same 64 bytes an outcome row name already carries, in the same place.
#[test]
fn vf2_d6_an_oversized_canon_source_is_refused_at_offer() {
    let (mut tw, e6) = boot_e6();
    let r = run_rig(&mut tw, 3_000);
    for (kind, source) in [
        (
            "scene.outcome".to_string(),
            "run:".to_string() + &"9".repeat(mush_economy::MAX_OUTCOME_NAME_BYTES),
        ),
        (
            "k".repeat(mush_economy::MAX_OUTCOME_NAME_BYTES + 1),
            "run:1".to_string(),
        ),
    ] {
        let mut spec = run_spec(
            "Extraction",
            r.johnson,
            (r.j_acct, 1_000),
            vec![PrincipalId::Object(r.johnson)],
            vec![row("success", vec![0], vec![exact(r.a_acct, 1_000)])],
        );
        spec.conditions = vec![TypedCondition::Attestation {
            by: AttestorSet::Canon {
                kind: SmolStr::new(&kind),
                source: Some(SmolStr::new(&source)),
            },
        }];
        spec.adjudicator = None;
        spec.evidence = vec![mush_economy::EvidenceKind::Attestation];
        e6.stage_offer(spec);
        ok(&mut tw, r.johnson, "@e6/offer");
        assert!(
            e6.offered().is_err(),
            "an oversized canon set was accepted at offer"
        );
        assert_eq!(
            snap(&tw).contracts().count(),
            0,
            "an oversized canon set was frozen into a surface"
        );
    }
}

/// **D7.** `@econ/history` collapses every masked **line** into one
/// `MASKED_ACCOUNTS` token precisely so a runner cannot count the other runners — and
/// then published the same arity through the hold half, one "a reservation … on
/// another account" per hidden hold. One token, the same way.
#[test]
fn vf2_d7_hidden_holds_collapse_into_one_line() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 10_000);
    // Two escrow placements the watcher cannot see, on one contract's own entry: the
    // Johnson's consideration and a second payer's.
    let faucet = snap(&tw)
        .accounts()
        .find(|x| matches!(x.owner, PrincipalId::System(_)))
        .expect("faucet")
        .id;
    ok(
        &mut tw,
        GOD,
        &format!("@econ/mint {faucet}={} 5000", r.a_acct),
    );
    // Staff draft it, so both payers' accounts are viewable at draft time; the
    // viewer below can see neither.
    ok(&mut tw, GOD, "@contract/draft Extraction");
    let id = only_contract(&tw);
    for who in [r.johnson, r.a] {
        ok(&mut tw, GOD, &format!("@contract/party {id}=#{}", who.0));
    }
    ok(
        &mut tw,
        GOD,
        &format!("@contract/pay {id}={} 1000", r.j_acct),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@contract/pay {id}={} 1000", r.a_acct),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@contract/split {id}={} 2000", r.b_acct),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@contract/remainder {id}={}", r.j_acct),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@contract/condition {id}=attest staff"),
    );
    ok(&mut tw, GOD, &format!("@contract/offer {id}"));
    for who in [GOD, r.johnson, r.a] {
        ok(&mut tw, who, &format!("@contract/accept {id}"));
    }
    // The settlement captures **both** escrow holds in one transaction and pays the
    // payee — so one entry in their history carries two hold effects, neither of
    // which is theirs to see.
    ok(&mut tw, GOD, &format!("@contract/attest {id}=0"));
    assert!(matches!(state(&tw, id), ContractState::Settled { .. }));

    // The payee reads their own account's history.
    let seen = ok(&mut tw, r.b, &format!("@econ/history {}", r.b_acct));
    assert_eq!(
        seen.matches("reservations on other accounts").count(),
        1,
        "the hidden holds published their arity: {seen}"
    );
    assert!(
        !seen.contains("a reservation"),
        "the old per-hold token is still emitted: {seen}"
    );
}

/// **D8.** `EconomicAsset` was raw `Deserialize`, so `StolenReport.reason` — and the
/// `Disputed` claim's, and the claim count — were bounded on the **write path
/// only**. A document with an over-long reason is refused by the wire now, and boot
/// skips it loudly rather than failing: every field above the latch is journal
/// authority, so the projection rebuilds from the journal and comes back frozen.
#[test]
fn vf2_d8_an_oversized_theft_report_reason_does_not_load() {
    let mut tw = boot();
    let owner = tw.spawn_player("Owner");
    ok(&mut tw, GOD, "@instrument/register nuyen=0");
    let obj = tw
        .world_mut()
        .create(ObjKind::Thing, "case", GOD, Some(owner))
        .expect("an object");
    ok(
        &mut tw,
        GOD,
        &format!("@asset/title #{}=#{}", obj.0, owner.0),
    );
    ok(
        &mut tw,
        owner,
        &format!("@asset/stolen #{}=public lifted", obj.0),
    );

    let mut kv = fold_kv(&mut tw);
    let key = format!("asset:{}", obj.0);
    patch(&mut kv, &key, |v| {
        if let Some(map) = v.as_object_mut()
            && let Some(report) = map.get_mut("stolen").and_then(|s| s.as_object_mut())
        {
            report.insert(
                "reason".to_owned(),
                serde_json::Value::String("x".repeat(mush_economy::MAX_REASON_BYTES + 1)),
            );
        }
    });
    // The wire refuses it outright…
    let raw = kv
        .iter()
        .find(|(k, _)| k.as_str() == key)
        .map(|(_, v)| v.clone())
        .expect("the asset doc");
    assert!(
        serde_json::from_value::<mush_economy::EconomicAsset>(raw).is_err(),
        "an over-long theft report reason still decodes"
    );
    // …and the boot survives it, rebuilding the title from the journal, frozen.
    reboot(&mut tw, kv);
    let a = snap(&tw).asset(obj).cloned().expect("the title survived");
    assert!(a.frozen, "the rebuilt title is not latched: {a:?}");
    assert_eq!(a.title_owner, PrincipalId::Object(owner));
}

/// **D10.** `@asset/list` walked **every** asset before the view gate, so one command
/// on a large world was one `may_view_asset` per title whatever the filter said. It
/// is bounded on records visited and pages from an `after=` cursor, and the cursor is
/// the last record *visited* rather than the last one shown — so a narrow viewer and
/// a staffer walk the same number of pages and truncate identically.
#[test]
fn vf2_d10_asset_list_is_bounded_on_records_visited_and_pages() {
    let mut tw = boot();
    let owner = tw.spawn_player("Owner");
    ok(&mut tw, GOD, "@instrument/register nuyen=0");
    let mut objs = Vec::new();
    for i in 0..(mush_economy::ASSET_PAGE_VISIT + 40) {
        let obj = tw
            .world_mut()
            .create(ObjKind::Thing, &format!("case {i}"), GOD, Some(owner))
            .expect("an object");
        ok(
            &mut tw,
            GOD,
            &format!("@asset/title #{}=#{}", obj.0, owner.0),
        );
        objs.push(obj);
    }
    let first = ok(&mut tw, GOD, "@asset/list");
    assert!(
        first.contains("@asset/list after=#"),
        "the page names no cursor: {first}"
    );
    // The cursor is the 256th record visited, not the 20th row rendered.
    let cursor = objs
        .get(mush_economy::ASSET_PAGE_VISIT - 1)
        .copied()
        .expect("the 256th title");
    assert!(
        first.contains(&format!("@asset/list after=#{}", cursor.0)),
        "the cursor is not the last record visited: {first}"
    );
    // The last page names no cursor at all.
    let last = ok(&mut tw, GOD, &format!("@asset/list after=#{}", cursor.0));
    assert!(
        !last.contains("after=#"),
        "the last page offers a cursor: {last}"
    );
}

/// **R2.** `owner_home` took its 256-vault budget **before** filtering by owner and
/// `Active`, so an organization whose own vault sorted past the bound resolved `None`:
/// its deposit stayed in the counterparty's vault, and the stranded page's own remedy
/// (`@vault/create`) minted a **higher** id, so the page could never be cleared.
#[test]
fn vf2_r2_owner_home_filters_by_owner_before_it_takes_its_budget() {
    let mut tw = boot();
    let owner = tw.spawn_player("Owner");
    ok(&mut tw, GOD, "@instrument/register nuyen=0");
    // 256 vaults belonging to somebody else, ahead of the one we are looking for.
    for i in 0..mush_economy::MAX_HOME_VAULT_VISIT {
        let backing = tw
            .world_mut()
            .create(ObjKind::Thing, &format!("box {i}"), GOD, None)
            .expect("a container");
        ok(
            &mut tw,
            GOD,
            &format!("@vault/create Foreign{i}=container:#{}", backing.0),
        );
    }
    let backing = tw
        .world_mut()
        .create(ObjKind::Thing, "the right box", GOD, None)
        .expect("a container");
    ok(
        &mut tw,
        GOD,
        &format!(
            "@vault/create Home=container:#{} owner=#{}",
            backing.0, owner.0
        ),
    );
    let home = mush_economy::owner_home(snap(&tw), &PrincipalId::Object(owner));
    assert_eq!(
        home.map(|(_, o)| o),
        Some(backing),
        "the owner's own vault sorted past the budget the foreign ones spent"
    );
}

/// **M8.** Re-verification **ORed** the condition's own attestor set with the
/// adjudicator's, where `@contract/attest` and the canon observer's `answers` both
/// **replace**. So an attestation neither writing path would ever have recorded —
/// one under the condition's own set on a surface that later named an adjudicator —
/// still re-verified, and settled a contract whose parties consented to one authority
/// and got another. Three sites, one rule (D-E6-8).
#[test]
fn vf2_m8_reverification_replaces_the_condition_set_with_the_adjudicators() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    ok(&mut tw, r.johnson, "@contract/draft Extraction");
    let id = only_contract(&tw);
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/party {id}=#{}", r.a.0),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/pay {id}={} 1000", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/split {id}={} 1000", r.a_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/remainder {id}={}", r.j_acct),
    );
    // The condition's own set names the payee; the surface's adjudicator names the
    // watcher, and it is the adjudicator that binds.
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/condition {id}=attest party:#{}", r.a.0),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/adjudicator {id}=party:#{}", r.watcher.0),
    );
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
    ok(&mut tw, r.a, &format!("@contract/accept {id}"));
    assert_eq!(state(&tw, id), ContractState::Active);

    // Patch in exactly the attestation the ORed rule accepted and the replacing rule
    // never writes: the payee's own word, under the condition's own set.
    let mut kv = fold_kv(&mut tw);
    patch(&mut kv, &format!("contract:{id}"), |v| {
        if let Some(map) = v.as_object_mut() {
            let divergent = mush_economy::Attestation {
                by: mush_economy::AttestorRef::Principal(PrincipalId::Object(r.a)),
                at: 1,
                clock: mush_economy::EconomicClock::RealUtc,
                source: SmolStr::new(format!("actor:{}", r.a.0)),
            };
            map.insert(
                "satisfied".to_owned(),
                serde_json::to_value(vec![Some(divergent)]).expect("serializes"),
            );
        }
    });
    reboot(&mut tw, kv);
    let before = bal(&tw, r.a_acct);
    sweep_at(&mut tw, 100_000);
    assert_eq!(
        bal(&tw, r.a_acct),
        before,
        "a divergent attestation satisfied a condition the adjudicator's set governs"
    );
    assert!(
        !matches!(state(&tw, id), ContractState::Settled { .. }),
        "the contract settled on it: {:?}",
        state(&tw, id)
    );
}

/// **M6.** `stranded_deposits` filtered on `r.to.is_none()` alone, while `returnable`
/// asked a *second* question — is the object still in the vault, is the home vault's
/// backing usable — so a deposit with a destination that could not actually be used
/// was dropped by the plan and reported by nothing at all. It is the complement of
/// `returnable` now, and the page names the object.
#[test]
fn vf2_m6_a_deposit_whose_object_left_the_vault_is_paged_not_dropped() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 3_000);
    let backing = tw
        .world_mut()
        .create(ObjKind::Thing, "strongbox", GOD, None)
        .expect("a container");
    ok(
        &mut tw,
        GOD,
        &format!(
            "@vault/create Box=container:#{} owner=#{}",
            backing.0, r.johnson.0
        ),
    );
    let vault = snap(&tw).vaults().map(|v| v.id).max().expect("a vault");
    let goods = tw
        .world_mut()
        .create(ObjKind::Thing, "crate", GOD, Some(r.a))
        .expect("an object");
    ok(
        &mut tw,
        GOD,
        &format!("@asset/title #{}=#{}", goods.0, r.a.0),
    );

    ok(&mut tw, r.johnson, "@contract/draft Delivery");
    let id = only_contract(&tw);
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/party {id}=#{}", r.a.0),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/pay {id}={} 1000", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/split {id}={} 1000", r.a_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/remainder {id}={}", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/condition {id}=deposit #{} {vault}", goods.0),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/deadline {id}=500 refund"),
    );
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
    ok(&mut tw, r.a, &format!("@contract/accept {id}"));
    ok(
        &mut tw,
        r.a,
        &format!("@vault/deposit #{}={vault}", goods.0),
    );
    assert_eq!(
        tw.world().get(goods).and_then(|o| o.location),
        Some(backing),
        "the deposit did not land"
    );

    // The object leaves the vault behind the contract's back — a staff move, a
    // destroyed container, anything the journal did not route. `deposit_returns`
    // still has a destination for it; `returnable` does not.
    tw.world_mut().move_to(goods, LIMBO).expect("the move");
    // The deadline unwinds it with a refund, so the only page on this subject is the
    // one the deposit earns.
    sweep_at(&mut tw, 1_000);
    assert_eq!(state(&tw, id), ContractState::Expired);

    let paged = tw
        .world()
        .module_state::<EconomyStore>()
        .expect("economy store")
        .escalations()
        .find(|e| e.subject == EscalationSubject::Contract(id))
        .cloned()
        .expect("a page about the deposit that could not go home");
    assert!(
        paged.context.contains(&format!("#{}", goods.0)),
        "the page does not name the object: {}",
        paged.context
    );
}

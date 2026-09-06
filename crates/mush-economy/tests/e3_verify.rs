//! E3 adversarial-verify regressions (the findings the E3a–E3d verify pass
//! confirmed, in the economy crate).
//!
//! Every test here is a hole that was open at `d8d17c3`, so each one is written as
//! *the exact sequence an attacker or an unlucky reboot would run*, not as a unit
//! probe of the fix:
//!
//! - **Authorization asked the wrong question** (A): it asked about each account's
//!   NET delta, and a zero delta reads as a credit — so a `[-1, +1]` line pair
//!   authorized itself.
//! - **The renderer hid the wrong resource** (B): a refusal naming an account other
//!   than the command's subject rendered raw, which distinguished "not there" from
//!   "not yours" for accounts the actor may see neither way.
//! - **The freeze was not durable** (H, I): the boot that froze also healed and
//!   persisted the drifted projections, so the NEXT boot came up clean and spending.
//! - **Control writes had no gate** (J) and a lag heal moved `revision` backwards (K).
//! - **An approver who no longer exists kept counting** (M).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use mush_core::module::{ModError, Registrar};
use mush_core::{
    ArgShape, Audience, CmdError, CommandSpec, Ctx, GOD, GameModule, LIMBO, ModuleBootData,
    ModuleWrite, ObjId, ObjKind,
};
use mush_economy::{
    APPROVAL_TTL_SECS, AccountId, Amount, ApprovalStatus, EconomicCause, EconomyError, EconomyPlan,
    EconomySnapshot, EconomyStore, IdempotencyKey, PostOutcome, RequestedLine,
};
use mush_org::{PrincipalId, SystemPrincipalId};
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, text_of};
use smol_str::SmolStr;

// ---- rig ------------------------------------------------------------------

fn boot() -> (TestWorld, Probe) {
    let probe = Probe::default();
    let tw = TestWorld::with_modules(vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(mush_economy::EconomyModule::with_custody(vec![])),
        Box::new(ProbeMod(probe.clone())),
    ]);
    (tw, probe)
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
fn revision(tw: &TestWorld, id: AccountId) -> u64 {
    snap(tw).account(id).expect("account").revision
}
fn journal_len(tw: &TestWorld) -> usize {
    store(tw).transactions().count()
}
fn thing(tw: &mut TestWorld, name: &str) -> ObjId {
    tw.world_mut()
        .create(ObjKind::Thing, name, GOD, Some(LIMBO))
        .expect("create")
}
fn account_of(tw: &TestWorld, owner: &PrincipalId) -> AccountId {
    snap(tw)
        .accounts()
        .find(|a| a.owner == *owner)
        .expect("account of owner")
        .id
}

/// Both namespaces, folded — an org reboot and an economy reboot are one event in
/// the real server.
type Kv = HashMap<SmolStr, Vec<(SmolStr, serde_json::Value)>>;

fn fold_kv(tw: &mut TestWorld, acc: &mut Kv) {
    for op in tw.take_module_ops() {
        let rows = acc.entry(op.ns.clone()).or_default();
        match op.op {
            ModuleWrite::JsonSet { key, value } => {
                rows.retain(|(k, _)| *k != key);
                rows.push((key, value));
            }
            ModuleWrite::Del { key } => rows.retain(|(k, _)| *k != key),
            ModuleWrite::StreamAppend { .. } => {}
        }
    }
}
/// Reboot the modules in place, so the world (and its objects) survive.
fn reboot(tw: &mut TestWorld, kv: &Kv) {
    let boot: HashMap<SmolStr, ModuleBootData> = kv
        .iter()
        .map(|(ns, rows)| {
            (
                ns.clone(),
                ModuleBootData {
                    kv: rows.clone(),
                    streams: HashMap::new(),
                    ..Default::default()
                },
            )
        })
        .collect();
    tw.modules_loaded(boot);
}
fn patch(kv: &mut Kv, key: &str, f: impl Fn(&mut serde_json::Value)) {
    let rows = kv
        .get_mut(&SmolStr::new_static("economy"))
        .expect("economy kv");
    let mut hit = false;
    for (k, v) in rows.iter_mut() {
        if k == key {
            f(v);
            hit = true;
        }
    }
    assert!(hit, "no economy doc {key:?} to patch");
}
fn put(kv: &mut Kv, key: &str, value: serde_json::Value) {
    let rows = kv.entry(SmolStr::new_static("economy")).or_default();
    rows.retain(|(k, _)| k != key);
    rows.push((SmolStr::new(key), value));
}

// ---- the probe: `post` has no command surface for a hand-built plan -------

#[derive(Default)]
struct ProbeState {
    econ: Option<EconomyPlan>,
    outcome: Option<Result<PostOutcome, EconomyError>>,
}
#[derive(Clone, Default)]
struct Probe(Arc<Mutex<ProbeState>>);

impl Probe {
    fn stage(&self, p: EconomyPlan) {
        self.0.lock().expect("probe lock").econ = Some(p);
    }
    fn outcome(&self) -> Result<PostOutcome, EconomyError> {
        self.0
            .lock()
            .expect("probe lock")
            .outcome
            .clone()
            .expect("an outcome was recorded")
    }
}

struct ProbeMod(Probe);

impl GameModule for ProbeMod {
    fn name(&self) -> &'static str {
        "probe"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(mush_core::CommandCompatibility::EconomyCarveOut);
        let st = self.0.clone();
        reg.command(
            CommandSpec {
                name: "@probe".into(),
                shape: ArgShape::EqPair,
                switches: ["post"].iter().copied().map(SmolStr::new).collect(),
                help: "test probe: post a staged EconomyPlan".into(),
                ..CommandSpec::default()
            },
            Arc::new(move |ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                let plan =
                    st.0.lock()
                        .expect("probe lock")
                        .econ
                        .take()
                        .expect("stage a plan first");
                // `post` is the last fallible call, as a real handler must treat it.
                let outcome = mush_economy::post(ctx, &plan);
                st.0.lock().expect("probe lock").outcome = Some(outcome);
                Ok(())
            }),
        )?;
        Ok(())
    }
}

fn plan_of(actor: ObjId, key: &str, lines: Vec<(AccountId, i64)>) -> EconomyPlan {
    EconomyPlan::new(
        IdempotencyKey::new("verify", key),
        PrincipalId::Object(actor),
        EconomicCause::Command { serial: 0 },
        lines
            .into_iter()
            .map(|(account, delta)| RequestedLine {
                account,
                delta: Amount::from_minor(delta),
                memo: SmolStr::default(),
            })
            .collect(),
    )
}

// ---- the fixture ----------------------------------------------------------

struct Fx {
    pat: ObjId,
    quinn: ObjId,
    faucet: AccountId,
    pat_acct: AccountId,
    quinn_acct: AccountId,
}

/// A faucet, two players with wallets, and 10 000 in Pat's pocket.
fn economy(tw: &mut TestWorld) -> Fx {
    let pat = tw.spawn_player("Pat");
    let quinn = tw.spawn_player("Quinn");
    ok(tw, GOD, "@instrument/register gold=0");
    ok(tw, GOD, "@account/system faucet=gold");
    ok(tw, GOD, &format!("@account/open #{}=gold", pat.0));
    ok(tw, GOD, &format!("@account/open #{}=gold", quinn.0));
    let faucet = account_of(
        tw,
        &PrincipalId::System(SystemPrincipalId::new("faucet").expect("id")),
    );
    let pat_acct = account_of(tw, &PrincipalId::Object(pat));
    let quinn_acct = account_of(tw, &PrincipalId::Object(quinn));
    ok(tw, GOD, &format!("@econ/mint {faucet}={pat_acct} 10000"));
    Fx {
        pat,
        quinn,
        faucet,
        pat_acct,
        quinn_acct,
    }
}

/// An org wallet on a fresh org's root scope: **active**, and invisible to anyone
/// without a capability on that compartment.
fn org_wallet(tw: &mut TestWorld, org: &str) -> AccountId {
    ok(tw, GOD, &format!("@org/create {org}"));
    ok(tw, GOD, &format!("@account/open org:{org}=gold"));
    snap(tw)
        .accounts()
        .filter(|a| a.scope.is_some())
        .map(|a| a.id)
        .max()
        .expect("the org wallet")
}

// ======================================================================
// A — authorization asked about the NET delta, and zero reads as a credit
// ======================================================================

#[test]
fn a_net_zero_line_pair_still_needs_authority_to_debit() {
    let (mut tw, probe) = boot();
    let fx = economy(&mut tw);
    let wallet = org_wallet(&mut tw, "Ironhold");
    let before_txs = journal_len(&tw);
    let before_rev = revision(&tw, wallet);

    // Pat holds NO capability on that compartment. The pair nets to zero, so the
    // old net-only question answered "this is a credit, and receiving is free".
    probe.stage(plan_of(fx.pat, "net-zero", vec![(wallet, -1), (wallet, 1)]));
    ok(&mut tw, fx.pat, "@probe/post");
    assert_eq!(
        probe.outcome(),
        Err(EconomyError::Unauthorized),
        "a net-zero pair posted without authority"
    );
    assert_eq!(
        journal_len(&tw),
        before_txs,
        "the refused plan journaled an entry"
    );
    assert_eq!(
        revision(&tw, wallet),
        before_rev,
        "the refused plan bumped the wallet's revision"
    );
    assert_eq!(bal(&tw, wallet), 0);
}

#[test]
fn a_larger_credit_does_not_excuse_a_debit_on_the_same_account() {
    let (mut tw, probe) = boot();
    let fx = economy(&mut tw);
    let wallet = org_wallet(&mut tw, "Ironhold");
    let before_txs = journal_len(&tw);

    // The wallet NETS a credit (+10) while a line really debits it (-10); Pat funds
    // the difference from their own account, so every other invariant is satisfied.
    probe.stage(plan_of(
        fx.pat,
        "credit-cover",
        vec![(wallet, -10), (wallet, 20), (fx.pat_acct, -10)],
    ));
    ok(&mut tw, fx.pat, "@probe/post");
    assert_eq!(
        probe.outcome(),
        Err(EconomyError::Unauthorized),
        "a debit line hid behind a larger credit on the same account"
    );
    assert_eq!(journal_len(&tw), before_txs);
    assert_eq!(
        bal(&tw, fx.pat_acct),
        10_000,
        "the refused plan moved money"
    );
}

#[test]
fn pay_refuses_an_account_paying_itself() {
    // The belt on the engine's gross-debit rule, refused before any lookup — so it
    // discloses nothing about either id.
    let (mut tw, _p) = boot();
    let fx = economy(&mut tw);
    let before_txs = journal_len(&tw);
    let seen = err(
        &mut tw,
        fx.pat,
        &format!("@econ/pay {}={} 1", fx.pat_acct, fx.pat_acct),
    );
    assert!(seen.contains("cannot pay itself"), "{seen}");
    assert_eq!(journal_len(&tw), before_txs);
    assert_eq!(revision(&tw, fx.pat_acct), revision(&tw, fx.pat_acct));
    // An id that does not exist gets the SAME text, so the belt is not an oracle.
    let missing = err(&mut tw, fx.pat, "@econ/pay 4242=4242 1");
    assert_eq!(
        seen, missing,
        "the self-pay refusal distinguished a real account from a missing one"
    );
}

// ======================================================================
// B — the existence-hiding renderer collapsed only against the SUBJECT
// ======================================================================

#[test]
fn a_pay_refusal_never_says_which_destination_it_is() {
    let (mut tw, _p) = boot();
    let fx = economy(&mut tw);
    let hidden = org_wallet(&mut tw, "Umbra");
    // Both real destinations are closed, so the refusal is an answer ABOUT them
    // ("account N is not active") rather than about Pat's own account.
    ok(&mut tw, GOD, &format!("@account/close {}", fx.quinn_acct));
    ok(&mut tw, GOD, &format!("@account/close {hidden}"));
    let before_txs = journal_len(&tw);

    let missing = err(
        &mut tw,
        fx.pat,
        &format!("@econ/pay {}=4242 1", fx.pat_acct),
    );
    let closed = err(
        &mut tw,
        fx.pat,
        &format!("@econ/pay {}={} 1", fx.pat_acct, fx.quinn_acct),
    );
    let wallet = err(
        &mut tw,
        fx.pat,
        &format!("@econ/pay {}={} 1", fx.pat_acct, hidden),
    );

    assert!(missing.contains("No such account."), "{missing}");
    assert_eq!(
        missing, closed,
        "a closed third-party account was distinguishable from a missing one"
    );
    assert_eq!(
        missing, wallet,
        "an org wallet Pat may not view was distinguishable from a missing account"
    );
    assert!(
        !closed.contains("not active"),
        "the raw refusal leaked: {closed}"
    );
    assert_eq!(journal_len(&tw), before_txs);
}

#[test]
fn a_maintenance_refusal_hides_the_wallet_it_was_pointed_at() {
    let (mut tw, _p) = boot();
    let fx = economy(&mut tw);
    // ACTIVE, org-owned, and invisible to Pat: pointing maintenance at it is refused
    // as "scope and owner do not agree", which is an answer about a wallet Pat is
    // not allowed to know exists.
    let hidden = org_wallet(&mut tw, "Umbra");
    let widget = thing(&mut tw, "widget");
    ok(
        &mut tw,
        GOD,
        &format!("@asset/title #{}=#{}", widget.0, fx.pat.0),
    );

    let missing = err(
        &mut tw,
        fx.pat,
        &format!("@asset/maintenance #{}=4242", widget.0),
    );
    let wallet = err(
        &mut tw,
        fx.pat,
        &format!("@asset/maintenance #{}={}", widget.0, hidden),
    );
    assert!(missing.contains("No such account."), "{missing}");
    assert_eq!(
        missing, wallet,
        "the hidden wallet leaked through invariant 14's refusal: {wallet}"
    );
    assert!(
        !wallet.contains("scope and owner"),
        "the raw refusal leaked: {wallet}"
    );
    assert!(
        snap(&tw)
            .asset(widget)
            .expect("titled")
            .maintenance_account
            .is_none(),
        "the refused maintenance stuck"
    );
}

// ======================================================================
// G — the stale-request retire ran before the caps that can refuse
// ======================================================================

#[test]
fn the_pending_cap_refuses_before_retiring_a_stale_request() {
    let (mut tw, _p) = boot();
    let fx = economy(&mut tw);
    ok(
        &mut tw,
        GOD,
        &format!("@account/approval {}=100 threshold:1", fx.pat_acct),
    );

    // One request, then let it age past its TTL.
    ok(
        &mut tw,
        fx.pat,
        &format!("@econ/pay {}={} 500", fx.pat_acct, fx.quinn_acct),
    );
    let stale = snap(&tw)
        .approvals()
        .next()
        .expect("the first request")
        .clone();
    let now = tw.world().now();
    tw.set_now(now + APPROVAL_TTL_SECS + 10);

    // Fill the per-requester cap with LIVE requests (distinct amounts = distinct
    // plans), then re-submit the aged-out plan: the caps must refuse it BEFORE the
    // retire, or RAM says `Expired` while the durable doc still says `Pending`.
    for i in 0..mush_economy::MAX_PENDING_PER_REQUESTER {
        let amount = 501 + i;
        ok(
            &mut tw,
            fx.pat,
            &format!("@econ/pay {}={} {amount}", fx.pat_acct, fx.quinn_acct),
        );
    }
    let seen = err(
        &mut tw,
        fx.pat,
        &format!("@econ/pay {}={} 500", fx.pat_acct, fx.quinn_acct),
    );
    assert!(
        seen.contains("too many pending approval requests"),
        "{seen}"
    );
    assert_eq!(
        snap(&tw)
            .approval(stale.id)
            .expect("the stale request")
            .status,
        ApprovalStatus::Pending,
        "the refused submission retired the stale request anyway (a RAM write a discarded persist cannot undo)"
    );
    assert_eq!(
        bal(&tw, fx.pat_acct),
        10_000,
        "nothing posted along the way"
    );
}

// ======================================================================
// H — the whole-economy freeze was RAM-only, so the next boot came up clean
// ======================================================================

#[test]
fn the_whole_economy_freeze_survives_the_boot_that_heals_the_drift() {
    let (mut tw, _p) = boot();
    let fx = economy(&mut tw);
    let mut kv = Kv::new();
    fold_kv(&mut tw, &mut kv);

    // Break conservation: boot 1 freezes, heals the drifted doc to the journal, and
    // persists everything it just healed.
    patch(&mut kv, &format!("account:{}", fx.pat_acct), |v| {
        v["posted_balance"] = serde_json::json!(999);
    });
    reboot(&mut tw, &kv);
    assert!(
        store(&tw).is_frozen(),
        "the drift did not freeze the economy"
    );

    // Boot 2 over exactly what boot 1 wrote. Nothing in the docs drifts any more —
    // the ONLY thing that can keep the economy frozen is the durable latch.
    fold_kv(&mut tw, &mut kv);
    reboot(&mut tw, &kv);
    assert!(
        store(&tw).is_frozen(),
        "the freeze evaporated on the boot after the heal"
    );

    // While frozen, the non-recovery control writes are refused too (finding L).
    err(&mut tw, GOD, &format!("@account/open #{}=gold", fx.quinn.0));
    err(
        &mut tw,
        GOD,
        &format!("@account/approval {}=100 threshold:1", fx.pat_acct),
    );
    // Clearing it is a staff decision, and only staff may make it.
    err(&mut tw, fx.pat, "@econ/unfreeze");
    assert!(store(&tw).is_frozen());
    ok(&mut tw, GOD, "@econ/unfreeze");
    assert!(
        !store(&tw).is_frozen(),
        "@econ/unfreeze did not clear the latch"
    );

    // ...and THAT is durable too.
    fold_kv(&mut tw, &mut kv);
    reboot(&mut tw, &kv);
    assert!(
        !store(&tw).is_frozen(),
        "the cleared latch came back at the next boot"
    );
    ok(&mut tw, GOD, &format!("@account/open #{}=gold", fx.quinn.0));
}

#[test]
fn an_asset_drift_leaves_that_asset_frozen_with_a_reason() {
    let (mut tw, _p) = boot();
    let (_fx, widget) = {
        let fx = economy(&mut tw);
        let widget = thing(&mut tw, "widget");
        ok(
            &mut tw,
            GOD,
            &format!("@asset/title #{}=#{}", widget.0, fx.pat.0),
        );
        (fx, widget)
    };
    let mut kv = Kv::new();
    fold_kv(&mut tw, &mut kv);
    // The doc claims the journal's own sequence but names a different owner.
    patch(&mut kv, &format!("asset:{}", widget.0), |v| {
        v["title_owner"] = serde_json::json!({ "Object": GOD.0 });
    });

    reboot(&mut tw, &kv);
    assert!(
        store(&tw).is_frozen(),
        "title drift must freeze the economy"
    );
    let a = snap(&tw).asset(widget).expect("titled").clone();
    assert!(
        a.frozen,
        "the drifted title itself was left unfrozen: nothing said which record to look at"
    );
    assert_eq!(a.frozen_reason.as_deref(), Some("drift"));

    // The trace survives the recovery: the per-asset latch is a doc write of its own.
    fold_kv(&mut tw, &mut kv);
    reboot(&mut tw, &kv);
    let a = snap(&tw).asset(widget).expect("titled").clone();
    assert!(
        a.frozen && a.frozen_reason.as_deref() == Some("drift"),
        "the per-asset latch was not persisted"
    );
}

// ======================================================================
// I — an allocator floor that lasted exactly one boot
// ======================================================================

#[test]
fn the_approval_allocator_floors_past_a_doc_it_cannot_read() {
    let (mut tw, _p) = boot();
    let fx = economy(&mut tw);
    ok(
        &mut tw,
        GOD,
        &format!("@account/approval {}=100 threshold:1", fx.pat_acct),
    );
    let mut kv = Kv::new();
    fold_kv(&mut tw, &mut kv);
    // A corrupt request doc (skipped as a leaf) plus a stale meta: id 7 was minted,
    // and nothing readable remembers it.
    put(
        &mut kv,
        "approval:7",
        serde_json::json!({ "not": "an approval request" }),
    );
    patch(&mut kv, "meta", |v| {
        v["next_approval_id"] = serde_json::json!(0)
    });

    reboot(&mut tw, &kv);
    ok(
        &mut tw,
        fx.pat,
        &format!("@econ/pay {}={} 500", fx.pat_acct, fx.quinn_acct),
    );
    let minted = snap(&tw)
        .approvals()
        .map(|r| r.id.0)
        .max()
        .expect("a request was minted");
    assert!(
        minted >= 8,
        "the allocator re-minted an id an unreadable doc already holds: #{minted}"
    );
}

// ======================================================================
// J — control-doc writes confirmed ahead of their own durable write
// ======================================================================

#[test]
fn control_doc_writes_ride_the_ordinary_durable_gate() {
    let (mut tw, _p) = boot();
    let fx = economy(&mut tw);
    let chest = thing(&mut tw, "chest");
    ok(
        &mut tw,
        GOD,
        &format!("@vault/create strongbox=container:#{}", chest.0),
    );

    // A policy write: gated, and NON-strict — it moves no money.
    let _ = tw.engine_mut().take_durable_gate();
    ok(
        &mut tw,
        GOD,
        &format!("@account/approval {}=100 threshold:1", fx.pat_acct),
    );
    assert_eq!(
        tw.engine_mut().take_durable_gate(),
        Some((GOD, false)),
        "@account/approval confirmed ahead of its own durable write"
    );

    let _ = tw.engine_mut().take_durable_gate();
    ok(&mut tw, GOD, "@vault/freeze 0");
    assert_eq!(
        tw.engine_mut().take_durable_gate(),
        Some((GOD, false)),
        "@vault/freeze confirmed ahead of its own durable write"
    );
}

// ======================================================================
// K — a lag heal moved `revision` BACKWARDS
// ======================================================================

#[test]
fn a_lag_heal_never_lowers_an_account_revision() {
    let (mut tw, _p) = boot();
    let fx = economy(&mut tw);
    ok(
        &mut tw,
        GOD,
        &format!("@econ/mint {}={} 25", fx.faucet, fx.pat_acct),
    );
    let live = revision(&tw, fx.pat_acct);
    assert!(
        live >= 3,
        "the fixture should have touched Pat's account twice: {live}"
    );
    let mut kv = Kv::new();
    fold_kv(&mut tw, &mut kv);
    // A companion write that landed a revision behind (the routine dropped-doc case
    // the retained-seal table exists to heal).
    patch(&mut kv, &format!("account:{}", fx.pat_acct), |v| {
        v["revision"] = serde_json::json!(1);
    });

    reboot(&mut tw, &kv);
    assert!(!store(&tw).is_frozen(), "a benign lag must not freeze");
    assert_eq!(
        revision(&tw, fx.pat_acct),
        live,
        "boot re-derived the balance but left `revision` behind: an expected_revisions guard \
         written before the reboot would pass against a state it never checked"
    );
}

// ======================================================================
// M — a destroyed named approver kept counting
// ======================================================================

/// The scenario, with the named approver optionally destroyed after they approve.
/// Returns Pat's balance and what the second submission said.
fn named_set_scenario(destroy_approver: bool) -> (i64, String) {
    let (mut tw, _p) = boot();
    let fx = economy(&mut tw);
    let alice = tw.spawn_player("Alice");
    ok(
        &mut tw,
        GOD,
        &format!("@account/approval {}=100 named:1:#{}", fx.pat_acct, alice.0),
    );
    ok(
        &mut tw,
        fx.pat,
        &format!("@econ/pay {}={} 500", fx.pat_acct, fx.quinn_acct),
    );
    let request = snap(&tw).approvals().next().expect("a request").id;
    ok(&mut tw, alice, &format!("@econ/approve {request}"));
    if destroy_approver {
        tw.world_mut().destroy(alice).expect("destroy the approver");
    }
    let seen = texts(&tw.run(
        fx.pat,
        &format!("@econ/pay {}={} 500", fx.pat_acct, fx.quinn_acct),
    ));
    (bal(&tw, fx.pat_acct), seen)
}

#[test]
fn a_destroyed_named_approver_stops_counting() {
    // The control: alive and named, the approval satisfies the policy and it posts.
    let (balance, seen) = named_set_scenario(false);
    assert_eq!(
        balance, 9_500,
        "a live named approval did not satisfy the policy: {seen}"
    );

    // Destroyed: `NamedSet` asks only about identity, and an id in a stored set
    // outlives the person — so the recorded approval kept counting forever.
    let (balance, seen) = named_set_scenario(true);
    assert_eq!(
        balance, 10_000,
        "a destroyed approver's recorded approval still posted the plan: {seen}"
    );
    assert!(seen.contains("Approval required"), "{seen}");
}

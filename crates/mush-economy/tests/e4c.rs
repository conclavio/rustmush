//! E4c acceptance (08c §6): **facts, events, evidence, and the disclosure filter**.
//!
//! The verify axis of this cut is *existence leaks, evidence leaks*, so the rhythms
//! here are:
//!
//! - **unknown is unsatisfied.** A fact nobody can answer never settles anything, and
//!   neither does a canon record that names the wrong front.
//! - **prose is not evidence.** A canon commit with no matching typed condition
//!   attests nothing at all (08e answer 6).
//! - **the write stands even when the settlement does not.** Canon's own append and
//!   the attestation ride one batch, and an internal refusal leaves both.
//! - **events say ids, never money.** Every captured event is checked against the
//!   amounts that produced it.
//! - **the counterparty is a stranger.** Every disclosure test asserts on what is
//!   *absent* from the render, not only on what is present.
//!
//! The canon **bridge** lives in `mush-server` (because `mush-economy` must not
//! depend on `mush-canon`) and is tested in `mush-server/tests/e4c_bridge.rs`; here
//! the `CanonRecorded` event is emitted exactly as core delivers it.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use mush_core::module::{ModError, Registrar, WorldEvent};
use mush_core::{
    ArgShape, Audience, CmdError, CommandSpec, Ctx, Deferred, Effects, GOD, GameModule, LIMBO,
    ModuleOp, ModuleWrite, ObjId, ObjKind, Outbox, World,
};
use mush_economy::{
    AccountId, AssetEffectKind, ContractId, ContractState, ContractStateKind, EconomicCause,
    EconomicEvent, EconomyObserver, EconomySnapshot, EconomyStore, FactProvider, FactState,
};
use mush_org::{CapabilityId, PrincipalId, SystemPrincipalId};
use mush_proto::OutputEvent;
use mush_testkit::{TestWorld, text_of};

const NS: &str = "economy";
/// The namespace the canon module writes its ledger chunk under. The economy never
/// depends on `mush-canon`, so these tests stand in for canon's append with a
/// foreign-namespace op queued on the same step — which is exactly what it is.
const CANON_NS: &str = "canon";

// ---- stubs ----------------------------------------------------------------

/// A fact provider with a fixed table, standing in for `mush-jobs`/`mush-sim`.
#[derive(Debug)]
struct StubFacts {
    /// `(kind, id) -> satisfied`. An absent entry is **unknown**, not false.
    table: Vec<((&'static str, &'static str), bool)>,
}

impl FactProvider for StubFacts {
    fn provider(&self) -> &str {
        "test"
    }
    fn resolve(&self, _w: &World, kind: &str, id: &str, revision: Option<u64>) -> Option<FactState> {
        // A provider may refuse a stale reference (doc 10 §15); this one knows no
        // revisions at all, so a reference claiming one is unknown.
        if revision.is_some() {
            return None;
        }
        self.table
            .iter()
            .find(|((k, i), _)| *k == kind && *i == id)
            .map(|(_, satisfied)| FactState { satisfied: *satisfied, actor: None, at: 10 })
    }
}

/// An observer that records everything it is told.
#[derive(Debug, Clone, Default)]
struct Recorder {
    seen: Arc<Mutex<Vec<EconomicEvent>>>,
}

impl EconomyObserver for Recorder {
    fn on_economic_event(&self, _ctx: &mut Ctx<'_>, ev: &EconomicEvent) {
        self.seen.lock().expect("recorder lock").push(ev.clone());
    }
}

/// A stand-in for the ruleset that owns a title issuer: it exposes
/// [`mush_economy::issue_title`] as a command so the seam is driven through real
/// dispatch, with a real write batch and the real refusal rendering.
struct TitleIssuingRuleset;

impl GameModule for TitleIssuingRuleset {
    fn name(&self) -> &'static str {
        "e4ctestruleset"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.command(
            CommandSpec {
                name: "@titleissue".into(),
                shape: ArgShape::EqPair,
                help: "@titleissue <obj>=<issuer> <owner>   test: mint title as a system principal"
                    .into(),
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                let bad = || CmdError::msg("Usage: @titleissue <obj>=<issuer> <owner>");
                let object = ObjId(
                    ctx.args.lhs().trim().trim_start_matches('#').parse().map_err(|_| bad())?,
                );
                let rhs = ctx.args.rhs().unwrap_or_default().to_string();
                let mut words = rhs.split_whitespace();
                let issuer = SystemPrincipalId::new(words.next().unwrap_or("")).ok_or_else(bad)?;
                let owner = ObjId(
                    words.next().unwrap_or("").trim_start_matches('#').parse().map_err(|_| bad())?,
                );
                mush_economy::issue_title(ctx, object, PrincipalId::Object(owner), None, issuer)
                    .map(|_| ())
                    .map_err(|e| CmdError::msg(e.to_string()))
            }),
        )?;
        // E4c verify fold (E4a): the whole-economy latch has a staff clear
        // (`@econ/unfreeze`) and no staff *set* — boot's recovery pass sets it. This
        // drives the seam the same way boot does.
        reg.command(
            CommandSpec {
                name: "@econfreeze".into(),
                shape: ArgShape::Raw,
                help: "@econfreeze   test: set the whole-economy latch".into(),
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                mush_economy::set_economy_frozen(ctx, true);
                Ok(())
            }),
        )?;
        Ok(())
    }
}

/// An observer that panics for its first `left` events (E4c verify fold, V6).
#[derive(Debug, Clone)]
struct Panicky {
    left: Arc<std::sync::atomic::AtomicU32>,
}

impl EconomyObserver for Panicky {
    fn on_economic_event(&self, _ctx: &mut Ctx<'_>, _ev: &EconomicEvent) {
        use std::sync::atomic::Ordering;
        if self.left.load(Ordering::SeqCst) > 0 {
            self.left.fetch_sub(1, Ordering::SeqCst);
            panic!("an observer panicked");
        }
    }
}

/// An observer that posts on every post, which is how a chain reaches the depth
/// cap (E4c verify fold, V6). Nothing in the tree does this today; the guard exists
/// for the day something does.
#[derive(Debug, Clone, Copy)]
struct Recurser;

impl EconomyObserver for Recurser {
    fn on_economic_event(&self, ctx: &mut Ctx<'_>, ev: &EconomicEvent) {
        let EconomicEvent::TransactionPosted { .. } = ev else { return };
        let Some((from, to, owner, seed)) = ctx.w.module_state::<EconomyStore>().and_then(|s| {
            let snap = s.snapshot();
            let mut it = snap.accounts().filter(|a| matches!(a.owner, PrincipalId::Object(_)));
            let a = it.next()?;
            let b = it.next()?;
            Some((a.id, b.id, a.owner.clone(), s.peek_next_transaction()))
        }) else {
            return;
        };
        let plan = mush_economy::EconomyPlan::new(
            mush_economy::IdempotencyKey::new(
                smol_str::SmolStr::new_static("test.recurse"),
                smol_str::SmolStr::new(seed.to_string()),
            ),
            owner,
            EconomicCause::Internal {
                kind: smol_str::SmolStr::new_static("test"),
                id: seed,
            },
            vec![
                mush_economy::RequestedLine {
                    account: from,
                    delta: mush_economy::Amount::from_minor(-1),
                    memo: smol_str::SmolStr::new_static("r"),
                },
                mush_economy::RequestedLine {
                    account: to,
                    delta: mush_economy::Amount::from_minor(1),
                    memo: smol_str::SmolStr::new_static("r"),
                },
            ],
        );
        let _ = mush_economy::post(ctx, &plan);
    }
}

// ---- rig ------------------------------------------------------------------

fn vocabulary() -> BTreeSet<CapabilityId> {
    mush_economy::CAPABILITIES
        .iter()
        .copied()
        .chain(["org.role.manage", "org.grant.delegate"])
        .map(|c| CapabilityId::new(c).expect("well-formed"))
        .collect()
}

fn boot_with(
    facts: Vec<Arc<dyn FactProvider>>,
    observers: Vec<Arc<dyn EconomyObserver>>,
    issuers: Vec<SystemPrincipalId>,
) -> TestWorld {
    TestWorld::with_modules(vec![
        Box::new(
            mush_org::OrgModule::new()
                .with_capabilities(vocabulary())
                .with_scope_use(vec![Arc::new(mush_economy::EconomyScopeUse)]),
        ),
        Box::new(
            mush_economy::EconomyModule::with_custody(vec![])
                .with_facts(facts)
                .with_observers(observers)
                .with_title_issuers(issuers)
                // E4c verify fold (E1): a canon attestor must name a
                // **producer-declared outcome kind**. These stand in for E6b's
                // `scene.outcome`; `scene.concluded` is deliberately NOT among them,
                // because it is a lifecycle record any participant can produce.
                .with_canon_outcome_kinds(vec![
                    smol_str::SmolStr::new_static("scene.outcome"),
                    smol_str::SmolStr::new_static("sim.journey.arrived"),
                ]),
        ),
        Box::new(TitleIssuingRuleset),
    ])
}

fn boot() -> TestWorld {
    boot_with(Vec::new(), Vec::new(), Vec::new())
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
fn state(tw: &TestWorld, id: ContractId) -> ContractState {
    snap(tw).contract(id).expect("contract").state
}
fn bal(tw: &TestWorld, id: AccountId) -> i64 {
    snap(tw).account(id).expect("account").posted_balance.minor()
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
fn thing(tw: &mut TestWorld, name: &str) -> ObjId {
    tw.world_mut().create(ObjKind::Thing, name, GOD, Some(LIMBO)).expect("create")
}
fn sweep_at(tw: &mut TestWorld, t: i64) {
    tw.set_now(t);
    let _ = tw.fire_due(t.saturating_mul(1000));
}

/// Emit one `CanonRecorded` **with a canon-namespace write already queued on the same
/// step**, which is exactly the shape `mush_canon::commit` produces: it persists,
/// then emits synchronously. Returns every module op the step queued.
fn record_canon(
    tw: &mut TestWorld,
    canon_id: u64,
    kind: &str,
    source: &str,
    entities: &[ObjId],
) -> Vec<ModuleOp> {
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    out.defer(Deferred::Persist(ModuleOp {
        ns: CANON_NS.into(),
        op: ModuleWrite::JsonSet {
            key: format!("log:canon:{canon_id}").into(),
            value: serde_json::json!({ "id": canon_id, "kind": kind, "source": source }),
        },
    }));
    tw.engine_mut().emit_world_event(
        &WorldEvent::CanonRecorded {
            canon_id,
            kind: kind.to_string(),
            entities: entities.to_vec(),
            source: source.to_string(),
        },
        &mut out,
        &mut fx,
    );
    tw.take_module_ops()
}

/// A Johnson, one runner, a watcher, an instrument and funded wallets.
struct Run {
    johnson: ObjId,
    runner: ObjId,
    watcher: ObjId,
    j_acct: AccountId,
    r_acct: AccountId,
}

fn run_rig(tw: &mut TestWorld, funds: i64) -> Run {
    let johnson = tw.spawn_player("Johnson");
    let runner = tw.spawn_player("Runner");
    let watcher = tw.spawn_player("Watcher");
    ok(tw, GOD, "@instrument/register nuyen=0");
    ok(tw, GOD, "@account/system faucet=nuyen");
    for who in [johnson, runner, watcher] {
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
    Run { johnson, runner, watcher, j_acct, r_acct: account_of(tw, runner) }
}

/// A one-condition run, offered and fully accepted. `condition` is the raw
/// `@contract/condition` right-hand side.
fn accepted_run(tw: &mut TestWorld, r: &Run, pay: i64, condition: &str) -> ContractId {
    ok(tw, r.johnson, "@contract/draft Extraction");
    let id = only_contract(tw);
    ok(tw, r.johnson, &format!("@contract/party {id}=#{}", r.runner.0));
    ok(tw, r.johnson, &format!("@contract/pay {id}={} {pay}", r.j_acct));
    ok(tw, r.johnson, &format!("@contract/split {id}={} {pay}", r.r_acct));
    ok(tw, r.johnson, &format!("@contract/remainder {id}={}", r.j_acct));
    ok(tw, r.johnson, &format!("@contract/condition {id}={condition}"));
    ok(tw, r.johnson, &format!("@contract/offer {id}"));
    ok(tw, r.johnson, &format!("@contract/accept {id}"));
    ok(tw, r.runner, &format!("@contract/accept {id}"));
    assert_eq!(state(tw, id), ContractState::Active, "the run did not come into force");
    id
}

// ======================================================================
// 1. The FactProvider seam
// ======================================================================

#[test]
fn a_provider_that_says_yes_lets_the_contract_settle() {
    let facts: Vec<Arc<dyn FactProvider>> =
        vec![Arc::new(StubFacts { table: vec![(("resolved", "7"), true)] })];
    let mut tw = boot_with(facts, Vec::new(), Vec::new());
    let r = run_rig(&mut tw, 1_000);
    let id = accepted_run(&mut tw, &r, 300, "fact test resolved 7");
    // Nothing attests a fact: it is re-resolved live, and the sweep is what notices.
    sweep_at(&mut tw, 100);
    assert!(
        matches!(state(&tw, id), ContractState::Settled { .. }),
        "a satisfied fact did not settle"
    );
    assert_eq!(bal(&tw, r.r_acct), 300, "the runner was not paid");
}

#[test]
fn a_false_fact_an_unknown_fact_and_an_unregistered_provider_all_leave_the_escrow_alone() {
    // `9` is known-false; `11` is not in the table at all, which is *unknown*; and
    // `nosuch` names a provider nothing wired.
    for fact in ["fact test resolved 9", "fact test resolved 11", "fact nosuch resolved 7"] {
        let facts: Vec<Arc<dyn FactProvider>> = vec![Arc::new(StubFacts {
            table: vec![(("resolved", "9"), false), (("resolved", "7"), true)],
        })];
        let mut tw = boot_with(facts, Vec::new(), Vec::new());
        let r = run_rig(&mut tw, 1_000);
        let id = accepted_run(&mut tw, &r, 300, fact);
        sweep_at(&mut tw, 100);
        assert_eq!(state(&tw, id), ContractState::Active, "`{fact}` settled something");
        assert_eq!(bal(&tw, r.r_acct), 0, "`{fact}` paid the runner");
    }
}

// ======================================================================
// 2. The canon attestation observer
// ======================================================================

#[test]
fn a_sealed_scene_outcome_attests_and_settles_in_one_batch() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 1_000);
    let id = accepted_run(&mut tw, &r, 300, "attest canon:scene.outcome:scene:5");
    let _ = tw.drain_write_batch();

    let ops = record_canon(&mut tw, 41, "scene.outcome", "scene:5", &[r.runner]);
    assert!(
        matches!(state(&tw, id), ContractState::Settled { .. }),
        "the sealed scene outcome did not settle the run"
    );
    assert_eq!(bal(&tw, r.r_acct), 300);

    // One batch: canon's own append, the economy's journal chunk, and the contract
    // document that records the evidence.
    assert!(ops.iter().any(|o| o.ns == CANON_NS), "canon's write is not in the batch");
    assert!(
        ops.iter().any(|o| o.ns == NS
            && matches!(&o.op, ModuleWrite::JsonSet { key, .. } if key.starts_with("log:journal"))),
        "the settlement's journal append is not in canon's batch"
    );
    let doc = format!("contract:{id}");
    assert!(
        ops.iter().any(
            |o| o.ns == NS && matches!(&o.op, ModuleWrite::JsonSet { key, .. } if key.as_str() == doc)
        ),
        "the attested contract document is not in canon's batch"
    );
    assert!(
        snap(&tw).contract(id).expect("contract").satisfied[0].is_some(),
        "the attestation was not recorded"
    );
}

#[test]
fn a_canon_record_naming_another_front_attests_nothing() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 1_000);
    let id = accepted_run(&mut tw, &r, 300, "attest canon:scene.outcome:scene:5");
    // Right kind, wrong source: E5's answer 20 wants a SPECIFIC front cited.
    let _ = record_canon(&mut tw, 41, "scene.outcome", "scene:6", &[r.runner]);
    assert_eq!(state(&tw, id), ContractState::Active, "another scene's outcome settled this run");
    assert!(snap(&tw).contract(id).expect("contract").satisfied[0].is_none());
    assert_eq!(bal(&tw, r.r_acct), 0);
}

#[test]
fn prose_alone_releases_no_escrow() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 1_000);
    let id = accepted_run(&mut tw, &r, 300, "attest canon:scene.outcome:scene:5");
    // The narrator commits an eloquent account of the same scene. It is canon, and
    // it is not the typed outcome the consent surface named.
    let _ = record_canon(&mut tw, 41, "scene.narration", "scene:5", &[r.runner]);
    let _ = record_canon(&mut tw, 42, "llm.summary", "scene:5", &[r.runner]);
    assert_eq!(state(&tw, id), ContractState::Active, "prose settled a run");
    assert!(snap(&tw).contract(id).expect("contract").satisfied[0].is_none());
    assert_eq!(bal(&tw, r.r_acct), 0);
}

#[test]
fn an_attestation_is_written_once_however_often_the_front_records_canon() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 1_000);
    // Two conditions, so the first canon record cannot settle the contract out from
    // under the second one.
    ok(&mut tw, r.johnson, "@contract/draft Voyage");
    let id = only_contract(&tw);
    ok(&mut tw, r.johnson, &format!("@contract/party {id}=#{}", r.runner.0));
    ok(&mut tw, r.johnson, &format!("@contract/pay {id}={} 300", r.j_acct));
    ok(&mut tw, r.johnson, &format!("@contract/split {id}={} 300", r.r_acct));
    ok(&mut tw, r.johnson, &format!("@contract/remainder {id}={}", r.j_acct));
    ok(&mut tw, r.johnson, &format!("@contract/condition {id}=attest canon:sim.journey.arrived:journey:9"));
    ok(&mut tw, r.johnson, &format!("@contract/condition {id}=attest party:#{}", r.johnson.0));
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
    ok(&mut tw, r.runner, &format!("@contract/accept {id}"));

    tw.set_now(100);
    let _ = record_canon(&mut tw, 1, "sim.journey.arrived", "journey:9", &[r.runner]);
    let first = snap(&tw).contract(id).expect("contract").satisfied[0].clone().expect("attested");
    tw.set_now(5_000);
    let _ = record_canon(&mut tw, 2, "sim.journey.arrived", "journey:9", &[r.runner]);
    let again = snap(&tw).contract(id).expect("contract").satisfied[0].clone().expect("attested");
    assert_eq!(first, again, "a second canon record re-stamped the attestation (an `After` anchor)");
}

#[test]
fn an_internal_refusal_inside_the_observer_leaves_canons_write_intact() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 1_000);
    // A sale: the Johnson buys a widget from the runner, and the runner's title is a
    // deliverable. Selling it elsewhere first makes every settlement refuse.
    let widget = thing(&mut tw, "widget");
    ok(&mut tw, GOD, &format!("@asset/title #{}=#{}", widget.0, r.runner.0));
    ok(&mut tw, r.johnson, "@contract/draft Sale");
    let id = only_contract(&tw);
    ok(&mut tw, r.johnson, &format!("@contract/party {id}=#{}", r.runner.0));
    ok(&mut tw, r.johnson, &format!("@contract/pay {id}={} 200", r.j_acct));
    ok(&mut tw, r.johnson, &format!("@contract/split {id}={} 200", r.r_acct));
    ok(&mut tw, r.johnson, &format!("@contract/remainder {id}={}", r.j_acct));
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/deliver {id}=title #{} #{}->#{}", widget.0, r.runner.0, r.johnson.0),
    );
    ok(&mut tw, r.johnson, &format!("@contract/condition {id}=attest canon:scene.outcome:scene:5"));
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
    ok(&mut tw, r.runner, &format!("@contract/accept {id}"));
    let third = tw.spawn_player("Third");
    ok(&mut tw, GOD, &format!("@asset/transfer #{}=#{}", widget.0, third.0));
    let _ = tw.drain_write_batch();

    let ops = record_canon(&mut tw, 41, "scene.outcome", "scene:5", &[r.runner]);
    // The settlement was refused — and neither canon's write nor the evidence went
    // with it.
    assert_eq!(
        state(&tw, id),
        ContractState::ReadyToSettle,
        "a refused settlement moved the contract on"
    );
    assert_eq!(bal(&tw, r.r_acct), 0, "a refused settlement paid out");
    assert!(ops.iter().any(|o| o.ns == CANON_NS), "an economy refusal took canon's write with it");
    assert!(
        snap(&tw).contract(id).expect("contract").satisfied[0].is_some(),
        "the attestation was rolled back by a settlement that refused"
    );
}

// ======================================================================
// 3. The EconomyObserver seam and the compact events
// ======================================================================

#[test]
fn economic_events_carry_ids_and_kinds_and_never_an_amount() {
    let rec = Recorder::default();
    let mut tw = boot_with(Vec::new(), vec![Arc::new(rec.clone())], Vec::new());
    let r = run_rig(&mut tw, 4_242);
    let id = accepted_run(&mut tw, &r, 1_234, "attest canon:scene.outcome:scene:5");
    let _ = record_canon(&mut tw, 41, "scene.outcome", "scene:5", &[r.runner]);
    assert!(matches!(state(&tw, id), ContractState::Settled { .. }));

    let seen = rec.seen.lock().expect("lock").clone();
    assert!(!seen.is_empty(), "no events were delivered");
    let json = serde_json::to_string(&seen).expect("events serialize");
    for amount in ["4242", "1234"] {
        assert!(!json.contains(amount), "an event carried an amount ({amount}): {json}");
    }
    assert!(!json.contains("Extraction"), "an event carried a contract name: {json}");
    assert!(!json.contains("escrow for contract"), "an event carried a description: {json}");
    assert!(!json.contains("Johnson"), "an event carried a party: {json}");
    assert!(
        seen.iter().any(|e| matches!(e, EconomicEvent::TransactionPosted { .. })),
        "no transaction event: {seen:?}"
    );
    assert!(
        seen.iter().any(|e| matches!(e, EconomicEvent::HoldChanged { .. })),
        "no hold event: {seen:?}"
    );
}

#[test]
fn a_settlement_event_carries_the_transaction_it_landed_on() {
    let rec = Recorder::default();
    let mut tw = boot_with(Vec::new(), vec![Arc::new(rec.clone())], Vec::new());
    let r = run_rig(&mut tw, 1_000);
    let id = accepted_run(&mut tw, &r, 300, "attest canon:scene.outcome:scene:5");
    let _ = record_canon(&mut tw, 41, "scene.outcome", "scene:5", &[r.runner]);
    let ContractState::Settled { transaction } = state(&tw, id) else {
        panic!("the run did not settle");
    };
    let seen = rec.seen.lock().expect("lock").clone();
    let settled: Vec<&EconomicEvent> = seen
        .iter()
        .filter(|e| {
            matches!(e, EconomicEvent::ContractChanged { state: ContractStateKind::Settled, .. })
        })
        .collect();
    assert_eq!(settled.len(), 1, "a settlement announced itself {} times", settled.len());
    assert!(
        matches!(settled[0], EconomicEvent::ContractChanged { transaction: Some(t), .. } if *t == transaction),
        "the settlement event does not key on its landed transaction: {:?}",
        settled[0]
    );
    // …and a transition with no money carries no transaction at all.
    assert!(
        seen.iter().any(|e| matches!(
            e,
            EconomicEvent::ContractChanged { state: ContractStateKind::Active, transaction: None, .. }
        )),
        "activation was not announced, or claimed a transaction: {seen:?}"
    );
}

// ======================================================================
// 4. The org audit wrapper
// ======================================================================

fn audit_entries(tw: &TestWorld) -> Vec<serde_json::Value> {
    tw.world()
        .module_state::<mush_org::OrgStore>()
        .expect("org store")
        .audit
        .open_entries(mush_org::AUDIT_LOG)
        .iter()
        .map(|e| e.value.clone())
        .collect()
}

#[test]
fn an_approval_decision_lands_in_the_org_audit_ledger() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 2_000);
    let ada = tw.spawn_player("Ada");
    ok(&mut tw, GOD, &format!("@account/approval {}=100 named:1:#{}", r.j_acct, ada.0));
    ok(&mut tw, r.johnson, &format!("@econ/pay {}={} 500", r.j_acct, r.r_acct));
    let request = snap(&tw).approvals().next().expect("a request was minted").id;
    ok(&mut tw, ada, &format!("@econ/approve {request}"));

    let entries = audit_entries(&tw);
    let approved = entries
        .iter()
        .find(|e| e["action"] == "economy.approval.approve")
        .unwrap_or_else(|| panic!("no approval audit entry: {entries:?}"));
    assert_eq!(approved["actor"], serde_json::json!(ada.0), "the audit names the wrong actor");
    assert_eq!(approved["detail"]["request"], serde_json::json!(request.0));
    assert_eq!(
        approved["detail"]["accounts"],
        serde_json::json!([r.j_acct.0]),
        "the audit does not name the account scope it decided over"
    );

    // A denial is audited too — the ledger keeps refusals, which is the point.
    ok(&mut tw, r.johnson, &format!("@econ/pay {}={} 400", r.j_acct, r.r_acct));
    let second = snap(&tw).approvals().find(|a| a.id != request).expect("a second request").id;
    ok(&mut tw, ada, &format!("@econ/deny {second}"));
    let entries = audit_entries(&tw);
    assert!(
        entries.iter().any(|e| e["action"] == "economy.approval.deny"
            && e["detail"]["request"] == serde_json::json!(second.0)),
        "the denial was not audited: {entries:?}"
    );
}

// ======================================================================
// 5. The disclosure filter
// ======================================================================

#[test]
fn history_shows_your_own_side_in_full_and_masks_a_counterparty_you_cannot_view() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 1_000);
    ok(&mut tw, r.johnson, &format!("@econ/pay {}={} 250", r.j_acct, r.r_acct));

    // The Johnson sees his own line with its id and its amount, and the runner's as
    // a stranger's.
    let seen = ok(&mut tw, r.johnson, &format!("@econ/history {}", r.j_acct));
    assert!(
        seen.contains(&format!("account #{}", r.j_acct)),
        "the viewer's own side is hidden: {seen}"
    );
    assert!(seen.contains("-250"), "the viewer's own amount is hidden: {seen}");
    assert!(seen.contains("other accounts"), "the counterparty was not masked: {seen}");
    assert!(
        !seen.contains(&format!("account #{}", r.r_acct)),
        "the counterparty's account id leaked: {seen}"
    );
    assert!(!seen.contains("250") || !seen.contains("+250"), "the masked side kept its amount: {seen}");

    // Staff read the same entry with both ids.
    let seen = ok(&mut tw, GOD, &format!("@econ/history {}", r.j_acct));
    assert!(
        seen.contains(&format!("account #{}", r.r_acct)),
        "staff cannot read the audit view: {seen}"
    );
}

#[test]
fn history_omits_staff_visibility_entries_entirely() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 1_000);
    ok(&mut tw, r.johnson, "@contract/draft Quiet");
    let id = only_contract(&tw);
    ok(&mut tw, r.johnson, &format!("@contract/party {id}=#{}", r.runner.0));
    ok(&mut tw, r.johnson, &format!("@contract/pay {id}={} 300", r.j_acct));
    ok(&mut tw, r.johnson, &format!("@contract/split {id}={} 300", r.r_acct));
    ok(&mut tw, r.johnson, &format!("@contract/remainder {id}={}", r.j_acct));
    ok(&mut tw, r.johnson, &format!("@contract/visibility {id}=staff"));
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));

    // The escrow placement is recorded `Staff`. The payer — a party! — is not shown
    // it, and is not shown a placeholder either.
    let seen = ok(&mut tw, r.johnson, &format!("@econ/history {}", r.j_acct));
    assert!(!seen.contains("escrow for contract"), "a staff-visibility entry was rendered: {seen}");
    assert!(!seen.to_lowercase().contains("hidden"), "a staff-visibility entry left a stub: {seen}");
    let staff = ok(&mut tw, GOD, &format!("@econ/history {}", r.j_acct));
    assert!(
        staff.contains("escrow for contract"),
        "staff cannot see the staff-visibility entry: {staff}"
    );
}

#[test]
fn history_renders_identically_for_an_unviewable_and_a_nonexistent_account() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 1_000);
    ok(&mut tw, r.johnson, &format!("@econ/pay {}={} 250", r.j_acct, r.r_acct));
    let unviewable = err(&mut tw, r.watcher, &format!("@econ/history {}", r.j_acct));
    let nonexistent = err(&mut tw, r.watcher, "@econ/history 4242");
    let unparsable = err(&mut tw, r.watcher, "@econ/history banana");
    assert_eq!(unviewable, nonexistent, "denied and nonexistent render differently");
    assert_eq!(unviewable, unparsable, "a bad id renders differently");
    assert!(unviewable.contains("No such account."), "{unviewable}");
}

#[test]
fn a_description_is_shown_to_a_party_and_not_to_a_watching_auditor() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 2_000);
    // The watcher is named on the Johnson's approval rule. While a request over that
    // account is live she may *view* it — and she is nobody's counterparty, so she is
    // not a party to anything that happened on it.
    ok(&mut tw, GOD, &format!("@account/approval {}=1000 named:1:#{}", r.j_acct, r.watcher.0));
    ok(&mut tw, r.johnson, &format!("@econ/pay {}={} 1500", r.j_acct, r.r_acct));
    assert!(snap(&tw).approvals().next().is_some(), "no live request minted the view standing");

    // An escrow placement under the threshold posts with a description of its own.
    let id = accepted_run(&mut tw, &r, 300, &format!("attest party:#{}", r.johnson.0));
    let _ = id;

    let party = ok(&mut tw, r.johnson, &format!("@econ/history {}", r.j_acct));
    assert!(party.contains("escrow for contract"), "a party was shown no description: {party}");
    let auditor = ok(&mut tw, r.watcher, &format!("@econ/history {}", r.j_acct));
    assert!(
        !auditor.contains("escrow for contract"),
        "a viewer who is not a party was shown the free text: {auditor}"
    );
    assert!(
        auditor.contains("reserved") || auditor.contains("other accounts"),
        "the auditor was not shown the entry at all: {auditor}"
    );
}

#[test]
fn history_is_newest_first_and_capped_at_twenty() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 10_000);
    for _ in 0..25 {
        ok(&mut tw, r.johnson, &format!("@econ/pay {}={} 1", r.j_acct, r.r_acct));
    }
    // D8: `n` is validated, not clamped, so the cap is what a bare call gives.
    err(&mut tw, GOD, &format!("@econ/history {} 100", r.j_acct));
    let seen = ok(&mut tw, GOD, &format!("@econ/history {}", r.j_acct));
    assert_eq!(seen.matches("tx #").count(), 20, "the cap was not applied: {seen}");
    let ids: Vec<u64> = seen
        .lines()
        .filter_map(|l| l.split("tx #").nth(1))
        .filter_map(|t| t.split(' ').next())
        .filter_map(|t| t.parse().ok())
        .collect();
    assert!(ids.first() > ids.last(), "history is not newest first: {ids:?}");
    // `n` is honoured below the cap.
    let three = ok(&mut tw, GOD, &format!("@econ/history {} 3", r.j_acct));
    assert_eq!(three.matches("tx #").count(), 3, "{three}");
}

#[test]
fn hold_show_masks_the_account_a_beneficiary_may_not_view() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 1_000);
    // A reservation on the Johnson's wallet, made out to the runner. The runner may
    // read the promise made to them and has no standing at all over the account it
    // is reserved against.
    ok(&mut tw, GOD, &format!("@hold/place {}=200 for=#{} advance", r.j_acct, r.runner.0));
    let hold = snap(&tw).holds().next().expect("hold").id;
    let seen = ok(&mut tw, r.runner, &format!("@hold/show {hold}"));
    assert!(seen.contains("an account"), "the payer's account was not masked: {seen}");
    assert!(
        !seen.contains(&format!("account #{}", r.j_acct)),
        "the payer's account id leaked through @hold/show: {seen}"
    );
    // The payer sees their own.
    let seen = ok(&mut tw, r.johnson, &format!("@hold/show {hold}"));
    assert!(seen.contains(&format!("account #{}", r.j_acct)), "{seen}");
}

// ======================================================================
// 6. Title issuance for registered system principals
// ======================================================================

fn issuer(name: &str) -> SystemPrincipalId {
    SystemPrincipalId::new(name).expect("valid system principal")
}

#[test]
fn a_registered_issuer_mints_title_and_the_journal_names_it() {
    let mut tw = boot_with(Vec::new(), Vec::new(), vec![issuer("wod.housing")]);
    let owner = tw.spawn_player("Tenant");
    let flat = thing(&mut tw, "flat");
    ok(&mut tw, GOD, &format!("@titleissue #{}=wod.housing #{}", flat.0, owner.0));

    let asset = snap(&tw).asset(flat).expect("the title record was not minted");
    assert_eq!(asset.title_owner, PrincipalId::Object(owner));
    let tx = store(&tw).transactions().last().expect("a transaction");
    assert_eq!(
        tx.actor,
        PrincipalId::System(issuer("wod.housing")),
        "the effect does not carry the issuer"
    );
    assert!(
        matches!(&tx.cause, EconomicCause::Internal { kind, id } if kind == "title.issue" && *id == u64::from(flat.0)),
        "the effect does not carry the title.issue cause: {:?}",
        tx.cause
    );
    assert_eq!(tx.asset_effects.len(), 1);
    assert_eq!(tx.asset_effects[0].kind, AssetEffectKind::Created);
    assert!(tx.lines.is_empty(), "issuing a title moved money");
}

#[test]
fn an_unregistered_issuer_mints_nothing() {
    let mut tw = boot_with(Vec::new(), Vec::new(), vec![issuer("wod.housing")]);
    let owner = tw.spawn_player("Tenant");
    let flat = thing(&mut tw, "flat");
    // Staff typing it changes nothing: the table is the authority, not the actor.
    err(&mut tw, GOD, &format!("@titleissue #{}=rogue.mint #{}", flat.0, owner.0));
    assert!(snap(&tw).asset(flat).is_none(), "an unregistered issuer left a title record");
    err(&mut tw, GOD, &format!("@titleissue #{}=e7.production #{}", flat.0, owner.0));
    assert!(snap(&tw).asset(flat).is_none(), "a second unregistered issuer was accepted");
    // …and the registered one still works on the same object.
    ok(&mut tw, GOD, &format!("@titleissue #{}=wod.housing #{}", flat.0, owner.0));
    assert!(snap(&tw).asset(flat).is_some());
}

// ======================================================================
// ---- E4c verify fold ----
//
// One test per confirmed finding. The rhythms are the cut's own: an oracle
// answers one message for every wrong id; evidence must come from somebody who
// could not have produced the settlement alone; and what a render withholds is
// asserted on as hard as what it shows.
// ======================================================================

/// D1. `@contract/deliver … obligation` was an account existence oracle: name any
/// id and read which of three messages came back.
#[test]
fn vf_d1_the_obligation_deliverable_answers_one_message_for_every_id_you_may_not_see() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 1_000);
    let faucet = snap(&tw)
        .accounts()
        .find(|x| matches!(x.owner, PrincipalId::System(_)))
        .expect("faucet")
        .id;
    let stranger = account_of(&tw, r.watcher);
    ok(&mut tw, r.johnson, "@contract/draft Lease");
    let id = only_contract(&tw);
    ok(&mut tw, r.johnson, &format!("@contract/party {id}=#{}", r.runner.0));

    let nonexistent = err(
        &mut tw,
        r.johnson,
        &format!("@contract/deliver {id}=obligation 4242 {} 50 every=600", r.j_acct),
    );
    let unviewable = err(
        &mut tw,
        r.johnson,
        &format!("@contract/deliver {id}=obligation {stranger} {} 50 every=600", r.j_acct),
    );
    // A faucet IS a standing mint, and saying so out loud identified it.
    let system = err(
        &mut tw,
        r.johnson,
        &format!("@contract/deliver {id}=obligation {faucet} {} 50 every=600", r.j_acct),
    );
    assert_eq!(nonexistent, unviewable, "an unviewable account is distinguishable from no account");
    assert_eq!(nonexistent, system, "a faucet is distinguishable from no account");
    assert!(nonexistent.contains("No such account."), "{nonexistent}");

    // Staff, who may see the faucet, get the real answer.
    ok(&mut tw, GOD, "@contract/draft Staffdraft");
    let staff_id = snap(&tw).contracts().map(|c| c.id).max().expect("contract");
    let seen = err(
        &mut tw,
        GOD,
        &format!("@contract/deliver {staff_id}=obligation {faucet} {} 50 every=600", r.j_acct),
    );
    assert!(seen.contains("standing mint"), "standing did not buy the real refusal: {seen}");

    // …and a party's own wallet is nobody's secret: the canonical lease still builds.
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/deliver {id}=obligation {} {} 50 every=600", r.r_acct, r.j_acct),
    );
}

/// D2. `@obligation/show|list` printed both account ids raw, to anybody who may
/// view *either* of them.
#[test]
fn vf_d2_obligation_renders_mask_the_account_the_viewer_may_not_see() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 1_000);
    ok(&mut tw, GOD, &format!("@obligation/create {}={} 50 every=600", r.j_acct, r.r_acct));
    let oid = snap(&tw).obligations().next().expect("obligation").id;

    let seen = ok(&mut tw, r.johnson, &format!("@obligation/show {oid}"));
    assert!(
        seen.contains(&format!("account #{}", r.j_acct)),
        "the debtor's own side is hidden: {seen}"
    );
    assert!(seen.contains("an account"), "the creditor's account was not masked: {seen}");
    assert!(
        !seen.contains(&format!("account #{}", r.r_acct)),
        "the creditor's account id leaked: {seen}"
    );
    // The amount stays: it is the debt the viewer owes.
    assert!(seen.contains("50"), "the viewer's own debt is hidden: {seen}");

    let listed = ok(&mut tw, r.johnson, "@obligation/list");
    assert!(listed.contains("an account"), "the list leaked what /show masks: {listed}");

    let staff = ok(&mut tw, GOD, &format!("@obligation/show {oid}"));
    assert!(
        staff.contains(&format!("account #{}", r.r_acct)),
        "staff lost the audit view: {staff}"
    );
}

/// D3. A reservation on an account is a fact its viewers may see; the contract
/// behind it is not. `@contract/show` omitted a `Staff` contract and `@hold/show`
/// handed the same person its id.
#[test]
fn vf_d3_a_reservation_is_visible_and_the_contract_behind_it_is_not() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 2_000);
    // The watcher may view the Johnson's wallet (a live named-approver request) and
    // is nobody's party.
    ok(&mut tw, GOD, &format!("@account/approval {}=1000 named:1:#{}", r.j_acct, r.watcher.0));
    ok(&mut tw, r.johnson, &format!("@econ/pay {}={} 1500", r.j_acct, r.r_acct));

    ok(&mut tw, r.johnson, "@contract/draft Blacklift");
    let id = only_contract(&tw);
    ok(&mut tw, r.johnson, &format!("@contract/party {id}=#{}", r.runner.0));
    ok(&mut tw, r.johnson, &format!("@contract/pay {id}={} 300", r.j_acct));
    ok(&mut tw, r.johnson, &format!("@contract/split {id}={} 300", r.r_acct));
    ok(&mut tw, r.johnson, &format!("@contract/remainder {id}={}", r.j_acct));
    ok(&mut tw, r.johnson, &format!("@contract/condition {id}=attest party:#{}", r.johnson.0));
    ok(&mut tw, r.johnson, &format!("@contract/visibility {id}=staff"));
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
    ok(&mut tw, r.runner, &format!("@contract/accept {id}"));
    let hold = snap(&tw).contract(id).expect("contract").escrow[0];

    let seen = ok(&mut tw, r.watcher, &format!("@hold/show {hold}"));
    assert!(seen.contains("300"), "the reservation itself was hidden: {seen}");
    assert!(seen.contains("held"), "the reservation's state was hidden: {seen}");
    for leak in ["contract", "economy.contract"] {
        assert!(!seen.contains(leak), "`{leak}` leaked through @hold/show: {seen}");
    }
    // …and the same through `@econ/history`: the placement is an effect-only entry
    // on a `Staff`-visibility transaction, so rule 4 is relaxed for it and the
    // description that names the contract is not.
    let hist = ok(&mut tw, r.watcher, &format!("@econ/history {}", r.j_acct));
    assert!(hist.contains("reserved"), "the placement was omitted entirely: {hist}");
    assert!(!hist.contains("escrow for contract"), "the contract leaked through history: {hist}");
    // Staff still read both.
    let staff = ok(&mut tw, GOD, &format!("@hold/show {hold}"));
    assert!(
        staff.contains(&format!("contract #{id}")),
        "staff lost the audit view: {staff}"
    );
}

/// D4. One `another account` per hidden line published the arity of a split, and
/// two of them were solvable by subtraction.
#[test]
fn vf_d4_consecutive_masked_lines_collapse_into_one_token() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 5_000);
    let mut runners = vec![r.runner, r.watcher];
    for n in 0..3 {
        let extra = tw.spawn_player(&format!("Extra{n}"));
        ok(&mut tw, GOD, &format!("@account/open #{}=nuyen", extra.0));
        runners.push(extra);
    }
    let accounts: Vec<AccountId> = runners.iter().map(|w| account_of(&tw, *w)).collect();

    ok(&mut tw, r.johnson, "@contract/draft Split");
    let id = only_contract(&tw);
    for who in &runners {
        ok(&mut tw, r.johnson, &format!("@contract/party {id}=#{}", who.0));
    }
    ok(&mut tw, r.johnson, &format!("@contract/pay {id}={} 500", r.j_acct));
    let shares: Vec<String> = accounts.iter().map(|a| format!("{a} 100")).collect();
    ok(&mut tw, r.johnson, &format!("@contract/split {id}={}", shares.join(" ")));
    ok(&mut tw, r.johnson, &format!("@contract/remainder {id}={}", r.j_acct));
    ok(&mut tw, r.johnson, &format!("@contract/condition {id}=attest party:#{}", r.johnson.0));
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    for who in std::iter::once(&r.johnson).chain(runners.iter()) {
        ok(&mut tw, *who, &format!("@contract/accept {id}"));
    }
    ok(&mut tw, r.johnson, &format!("@contract/attest {id}=0"));
    assert!(
        matches!(state(&tw, id), ContractState::Settled { .. }),
        "the five-way split did not settle"
    );

    // One runner reads their own account. They see their own line, and exactly one
    // token for everybody else — not four, which would have counted the split.
    let seen = ok(&mut tw, runners[0], &format!("@econ/history {}", accounts[0]));
    assert_eq!(seen.matches("other accounts").count(), 1, "masked arity was published: {seen}");
    assert!(seen.contains(&format!("account #{}", accounts[0])), "own side hidden: {seen}");
    for other in &accounts[1..] {
        assert!(
            !seen.contains(&format!("account #{other} ")),
            "another runner's id leaked: {seen}"
        );
    }
}

/// D5. `may_view_contract` admits an auditor holding `economy.account.view` on the
/// scope; `show_class` was treating that as consent to the terms.
#[test]
fn vf_d5_an_account_auditor_gets_a_notice_and_a_contract_officer_gets_the_terms() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 2_000);
    let auditor = tw.spawn_player("Auditor");
    let officer = tw.spawn_player("Officer");
    ok(&mut tw, GOD, "@org/create Ironhold");
    ok(&mut tw, GOD, "@org/scope create Ironhold=operations");
    let scope = {
        let st = tw.world().module_state::<mush_org::OrgStore>().expect("org store").state();
        let org = st.orgs().next().expect("the org").id;
        st.scopes_in(org).find(|s| s.name == "operations").expect("scope").id
    };
    ok(
        &mut tw,
        GOD,
        &format!("@org/grant #{}=Ironhold economy.account.view scope=#{}", auditor.0, scope.0),
    );
    ok(
        &mut tw,
        GOD,
        &format!("@org/grant #{}=Ironhold contract.accept scope=#{}", officer.0, scope.0),
    );

    ok(&mut tw, GOD, &format!("@account/open org:Ironhold/{}=nuyen", scope.0));
    let org_acct = snap(&tw).accounts().find(|a| a.scope == Some(scope)).expect("org account").id;
    let faucet = snap(&tw)
        .accounts()
        .find(|x| matches!(x.owner, PrincipalId::System(_)))
        .expect("faucet")
        .id;
    ok(&mut tw, GOD, &format!("@econ/mint {faucet}={org_acct} 1000"));

    ok(&mut tw, GOD, &format!("@contract/draft Supply scope=#{}", scope.0));
    let id = only_contract(&tw);
    ok(&mut tw, GOD, &format!("@contract/party {id}=#{}", r.runner.0));
    ok(&mut tw, GOD, &format!("@contract/pay {id}={org_acct} 400"));
    ok(&mut tw, GOD, &format!("@contract/split {id}={} 400", r.r_acct));
    ok(&mut tw, GOD, &format!("@contract/remainder {id}={org_acct}"));
    ok(&mut tw, GOD, &format!("@contract/condition {id}=attest staff"));
    ok(&mut tw, GOD, &format!("@contract/offer {id}"));

    let audited = ok(&mut tw, auditor, &format!("@contract/show {id}"));
    assert!(audited.contains("notice"), "an account auditor read the terms: {audited}");
    assert!(!audited.contains("400"), "an account auditor read the money: {audited}");
    let read = ok(&mut tw, officer, &format!("@contract/show {id}"));
    assert!(read.contains("400"), "a contract officer lost the terms: {read}");
}

/// D6. The notice class leaked through `/list`'s party count and through
/// `render_state`'s settlement transaction id.
#[test]
fn vf_d6_a_notice_viewer_reads_no_party_count_and_no_transaction_id() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 2_000);
    ok(&mut tw, r.johnson, "@contract/draft Raid");
    let id = only_contract(&tw);
    ok(&mut tw, r.johnson, &format!("@contract/party {id}=#{}", r.runner.0));
    ok(&mut tw, r.johnson, &format!("@contract/pay {id}={} 400", r.j_acct));
    ok(&mut tw, r.johnson, &format!("@contract/split {id}={} 400", r.r_acct));
    ok(&mut tw, r.johnson, &format!("@contract/remainder {id}={}", r.j_acct));
    ok(&mut tw, r.johnson, &format!("@contract/condition {id}=attest party:#{}", r.johnson.0));
    ok(&mut tw, r.johnson, &format!("@contract/visibility {id}=audience #{}", r.watcher.0));
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
    ok(&mut tw, r.runner, &format!("@contract/accept {id}"));

    let listed = ok(&mut tw, r.watcher, "@contract/list");
    assert!(!listed.contains("part"), "the audience read the party count: {listed}");

    ok(&mut tw, r.johnson, &format!("@contract/attest {id}=0"));
    let ContractState::Settled { transaction } = state(&tw, id) else { panic!("did not settle") };
    let shown = ok(&mut tw, r.watcher, &format!("@contract/show {id}"));
    let listed = ok(&mut tw, r.watcher, "@contract/list");
    for seen in [&shown, &listed] {
        assert!(seen.contains("settled"), "the audience was not told it settled: {seen}");
        assert!(
            !seen.contains(&format!("transaction #{transaction}")),
            "the settlement transaction id leaked to a notice viewer: {seen}"
        );
    }
}

/// D8. `n` was clamped, so `0` and `banana` produced two different answers, and
/// the help said nothing about what the renders withhold.
#[test]
fn vf_d8_history_validates_n_and_the_help_says_ids_are_masked() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 1_000);
    let zero = err(&mut tw, r.johnson, &format!("@econ/history {} 0", r.j_acct));
    let word = err(&mut tw, r.johnson, &format!("@econ/history {} banana", r.j_acct));
    let over = err(&mut tw, r.johnson, &format!("@econ/history {} 21", r.j_acct));
    assert_eq!(zero, word, "`0` and a word render differently");
    assert_eq!(zero, over, "`0` and an oversized n render differently");
    assert!(zero.contains("1-20"), "{zero}");

    let help = ok(&mut tw, r.johnson, "help @hold");
    assert!(help.contains("masked by your"), "@hold help does not state the masking rule: {help}");
    let help = ok(&mut tw, r.johnson, "help @obligation");
    assert!(help.contains("masked by your"), "@obligation help does not state the rule: {help}");
}

/// E1. `scene.concluded` is a *lifecycle* record any participant can produce, so a
/// payee gated on one could seal their own settlement. A canon attestor must name a
/// **producer-declared outcome kind**, and the record must be about a party.
#[test]
fn vf_e1_only_a_registered_outcome_kind_naming_a_party_attests() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 2_000);
    ok(&mut tw, r.johnson, "@contract/draft Extraction");
    let id = only_contract(&tw);
    ok(&mut tw, r.johnson, &format!("@contract/party {id}=#{}", r.runner.0));
    ok(&mut tw, r.johnson, &format!("@contract/pay {id}={} 300", r.j_acct));
    ok(&mut tw, r.johnson, &format!("@contract/split {id}={} 300", r.r_acct));
    ok(&mut tw, r.johnson, &format!("@contract/remainder {id}={}", r.j_acct));
    ok(&mut tw, r.johnson, &format!("@contract/condition {id}=attest canon:scene.concluded:scene:5"));
    let refused = err(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    assert!(refused.contains("outcome kind"), "a lifecycle kind was offerable: {refused}");

    // The registered kind is what a consent surface may name — and a record about
    // somebody else's operation still attests nothing.
    let mut tw = boot();
    let r = run_rig(&mut tw, 2_000);
    let id = accepted_run(&mut tw, &r, 300, "attest canon:scene.outcome:scene:5");
    let stranger = tw.spawn_player("Stranger");
    let _ = record_canon(&mut tw, 7, "scene.outcome", "scene:5", &[stranger]);
    assert_eq!(state(&tw, id), ContractState::Active, "an outcome about strangers settled the run");
    assert!(snap(&tw).contract(id).expect("contract").satisfied[0].is_none());
    // Naming a party is the third gate, and it is what answers the condition.
    let _ = record_canon(&mut tw, 8, "scene.outcome", "scene:5", &[r.runner]);
    assert!(
        matches!(state(&tw, id), ContractState::Settled { .. }),
        "a party's outcome did not settle"
    );
}

/// E2. `Canon { source: None }` accepted every record of its kind — the broad match
/// E5's owner answer 20 exists to refuse.
#[test]
fn vf_e2_a_canon_attestor_must_name_its_source() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 1_000);
    ok(&mut tw, r.johnson, "@contract/draft Extraction");
    let id = only_contract(&tw);
    let refused =
        err(&mut tw, r.johnson, &format!("@contract/condition {id}=attest canon:scene.outcome"));
    assert!(refused.contains("Usage"), "a sourceless canon attestor parsed: {refused}");
    err(&mut tw, r.johnson, &format!("@contract/adjudicator {id}=canon:scene.outcome"));

    // And a wrong source attests nothing (the property the wildcard destroyed).
    let mut tw = boot();
    let r = run_rig(&mut tw, 1_000);
    let id2 = accepted_run(&mut tw, &r, 300, "attest canon:scene.outcome:scene:5");
    let _ = record_canon(&mut tw, 9, "scene.outcome", "scene:6", &[r.runner]);
    assert_eq!(state(&tw, id2), ContractState::Active, "another front's outcome settled this run");
    assert!(snap(&tw).contract(id2).expect("contract").satisfied[0].is_none());
}

/// E4. Canon evidence is edge-triggered, and a missed edge was unrecoverable: the
/// escrow sat until a deadline the surface may not even carry.
#[test]
fn vf_e4_a_missed_canon_edge_is_recovered_by_unfreeze_and_by_staff_resolve() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 2_000);
    let id = accepted_run(&mut tw, &r, 300, "attest canon:scene.outcome:scene:5");
    // The record lands while the economy is frozen: the observer deliberately does
    // nothing with it, because a frozen economy must not act on evidence it cannot
    // settle.
    ok(&mut tw, GOD, "@econfreeze");
    let _ = record_canon(&mut tw, 11, "scene.outcome", "scene:5", &[r.runner]);
    assert!(snap(&tw).contract(id).expect("contract").satisfied[0].is_none());
    ok(&mut tw, GOD, "@econ/unfreeze");
    assert!(
        matches!(state(&tw, id), ContractState::Settled { .. }),
        "unfreeze did not re-scan the canon tail"
    );
    assert_eq!(bal(&tw, r.r_acct), 300);

    // The other half: an edge that is gone for good, resolved by staff at level 2.
    let mut tw = boot();
    let r = run_rig(&mut tw, 2_000);
    let id = accepted_run(&mut tw, &r, 300, "attest canon:scene.outcome:scene:99");
    ok(&mut tw, r.runner, &format!("@contract/dispute {id}"));
    ok(&mut tw, GOD, &format!("@contract/resolve {id}=settle the scene was never sealed"));
    assert!(
        matches!(state(&tw, id), ContractState::Settled { .. }),
        "staff could not resolve a contract whose canon edge was missed"
    );
    assert_eq!(bal(&tw, r.r_acct), 300);
}

/// E5. An `After` anchored on a deterministic condition never matured, because a
/// deterministic condition never filled its satisfaction slot — so there was
/// nothing to count the delay from.
#[test]
fn vf_e5_after_can_anchor_on_a_deterministic_condition() {
    let facts: Vec<Arc<dyn FactProvider>> =
        vec![Arc::new(StubFacts { table: vec![(("resolved", "7"), true)] })];
    let mut tw = boot_with(facts, Vec::new(), Vec::new());
    let r = run_rig(&mut tw, 2_000);
    ok(&mut tw, r.johnson, "@contract/draft Retainer");
    let id = only_contract(&tw);
    ok(&mut tw, r.johnson, &format!("@contract/party {id}=#{}", r.runner.0));
    ok(&mut tw, r.johnson, &format!("@contract/pay {id}={} 300", r.j_acct));
    ok(&mut tw, r.johnson, &format!("@contract/split {id}={} 300", r.r_acct));
    ok(&mut tw, r.johnson, &format!("@contract/remainder {id}={}", r.j_acct));
    // "Pay 60 s after the job is approved."
    ok(&mut tw, r.johnson, &format!("@contract/condition {id}=fact test resolved 7"));
    ok(&mut tw, r.johnson, &format!("@contract/condition {id}=after 0 60"));
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
    ok(&mut tw, r.runner, &format!("@contract/accept {id}"));

    sweep_at(&mut tw, 1_000);
    assert_eq!(state(&tw, id), ContractState::Active, "the delay was not honoured");
    let stamp = snap(&tw).contract(id).expect("contract").satisfied[0]
        .clone()
        .expect("the fact was never observed");
    assert_eq!(stamp.at, 1_000, "the observation was not stamped when the fact came true");

    sweep_at(&mut tw, 1_030);
    assert_eq!(state(&tw, id), ContractState::Active, "the delay matured early");
    sweep_at(&mut tw, 1_100);
    assert!(
        matches!(state(&tw, id), ContractState::Settled { .. }),
        "the delay never matured"
    );
    assert_eq!(bal(&tw, r.r_acct), 300);
}

/// E6. `issue_title` minted title for whatever `ObjId` it was handed.
#[test]
fn vf_e6_issue_title_refuses_an_object_that_is_not_in_the_world() {
    let issuer = SystemPrincipalId::new("wod.housing").expect("valid");
    let mut tw = boot_with(Vec::new(), Vec::new(), vec![issuer]);
    let owner = tw.spawn_player("Owner");
    err(&mut tw, GOD, &format!("@titleissue 424242=wod.housing #{}", owner.0));
    assert!(snap(&tw).assets().next().is_none(), "a ghost asset was created");

    // A destroyed object is refused for the same reason.
    let flat = thing(&mut tw, "flat");
    let _ = tw.world_mut().destroy(flat);
    err(&mut tw, GOD, &format!("@titleissue {}=wod.housing #{}", flat.0, owner.0));
    assert!(snap(&tw).assets().next().is_none(), "title was minted for a destroyed object");

    // …and a live one still works.
    let real = thing(&mut tw, "real flat");
    ok(&mut tw, GOD, &format!("@titleissue {}=wod.housing #{}", real.0, owner.0));
    assert!(snap(&tw).asset(real).is_some(), "a live object could not be titled");
}

/// E7. A re-issue after a write-off resolved to the first transaction and minted
/// nothing, while reporting `AlreadyPosted` — which reads as success.
#[test]
fn vf_e7_a_reissue_after_a_write_off_mints_the_next_generation() {
    let issuer = SystemPrincipalId::new("wod.housing").expect("valid");
    let mut tw = boot_with(Vec::new(), Vec::new(), vec![issuer]);
    let owner = tw.spawn_player("Owner");
    let flat = thing(&mut tw, "flat");
    ok(&mut tw, GOD, &format!("@titleissue {}=wod.housing #{}", flat.0, owner.0));
    assert_eq!(
        snap(&tw).asset(flat).expect("asset").lifecycle,
        mush_economy::AssetLifecycle::Active
    );
    ok(&mut tw, GOD, &format!("@asset/writeoff #{}=condemned", flat.0));
    assert_ne!(
        snap(&tw).asset(flat).expect("asset").lifecycle,
        mush_economy::AssetLifecycle::Active,
        "the write-off did not take"
    );

    ok(&mut tw, GOD, &format!("@titleissue {}=wod.housing #{}", flat.0, owner.0));
    assert_eq!(
        snap(&tw).asset(flat).expect("asset").lifecycle,
        mush_economy::AssetLifecycle::Active,
        "the re-issue was swallowed as an idempotency hit"
    );
    // Two `Created` effects in the journal: one per generation.
    let created = store(&tw)
        .transactions()
        .flat_map(|t| t.asset_effects.iter())
        .filter(|e| e.object == flat && e.kind == AssetEffectKind::Created)
        .count();
    assert_eq!(created, 2, "the second generation was never minted");
}

/// E8. Two providers claiming one namespace silently shadowed: the second answered
/// nothing, and every contract gated on it waited forever with no error anywhere.
#[test]
#[should_panic(expected = "two fact providers claim the namespace")]
fn vf_e8_duplicate_fact_provider_namespaces_are_a_wiring_panic() {
    let a: Arc<dyn FactProvider> = Arc::new(StubFacts { table: Vec::new() });
    let b: Arc<dyn FactProvider> = Arc::new(StubFacts { table: Vec::new() });
    let _ = mush_economy::EconomyModule::new().with_facts(vec![a, b]);
}

/// E9. A live contract waiting on evidence nobody will ever produce waited in
/// silence, with its escrow reserved and no deadline to end it.
#[test]
fn vf_e9_a_contract_with_no_condition_progress_is_paged() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 2_000);
    let id = accepted_run(&mut tw, &r, 300, "attest canon:scene.outcome:scene:5");
    sweep_at(&mut tw, 100);
    assert!(store(&tw).escalations().next().is_none(), "a fresh contract was paged");

    sweep_at(&mut tw, mush_economy::STALE_CONTRACT_AGE_SECS + 1_000);
    let paged: Vec<_> = store(&tw)
        .escalations()
        .filter(|e| e.subject == mush_economy::EscalationSubject::Contract(id))
        .collect();
    assert_eq!(paged.len(), 1, "a stale contract was not paged exactly once");
    assert!(
        paged[0].recommended.iter().any(|c| c.contains("@contract/resolve")),
        "the page does not recommend the command that ends it: {:?}",
        paged[0].recommended
    );
    // …and it stays one record, not one per fire.
    sweep_at(&mut tw, mush_economy::STALE_CONTRACT_AGE_SECS + 1_100);
    assert_eq!(store(&tw).escalations().count(), 1, "the page re-minted");
}

/// E10. One canon record could answer any number of contracts, and every one of
/// them settled inside the observer, on canon's own write batch.
#[test]
fn vf_e10_the_observer_settles_at_most_eight_and_leaves_the_rest_attested() {
    let mut tw = boot();
    let r = run_rig(&mut tw, 20_000);
    let n = mush_economy::MAX_OBSERVER_SETTLEMENTS_PER_EVENT + 2;
    let mut ids = Vec::new();
    for _ in 0..n {
        ok(&mut tw, r.johnson, "@contract/draft Job");
        let id = snap(&tw).contracts().map(|c| c.id).max().expect("contract");
        ok(&mut tw, r.johnson, &format!("@contract/party {id}=#{}", r.runner.0));
        ok(&mut tw, r.johnson, &format!("@contract/pay {id}={} 100", r.j_acct));
        ok(&mut tw, r.johnson, &format!("@contract/split {id}={} 100", r.r_acct));
        ok(&mut tw, r.johnson, &format!("@contract/remainder {id}={}", r.j_acct));
        ok(
            &mut tw,
            r.johnson,
            &format!("@contract/condition {id}=attest canon:scene.outcome:scene:5"),
        );
        ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
        ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
        ok(&mut tw, r.runner, &format!("@contract/accept {id}"));
        ids.push(id);
    }
    let _ = record_canon(&mut tw, 21, "scene.outcome", "scene:5", &[r.runner]);

    // Every one of them is attested — evidence is never dropped …
    for id in &ids {
        assert!(
            snap(&tw).contract(*id).expect("contract").satisfied[0].is_some(),
            "contract {id} lost its evidence to the cap"
        );
    }
    // …and the cap bounded how many the observer paid out in its own step.
    let settled =
        ids.iter().filter(|id| matches!(state(&tw, **id), ContractState::Settled { .. })).count();
    assert_eq!(
        settled,
        mush_economy::MAX_OBSERVER_SETTLEMENTS_PER_EVENT,
        "the per-event settlement cap did not bind"
    );
    // The sweep finishes the rest, on its own budget and its own batch.
    sweep_at(&mut tw, 100);
    for id in &ids {
        assert!(
            matches!(state(&tw, *id), ContractState::Settled { .. }),
            "the sweep did not finish contract {id}"
        );
    }
}

/// V6. The observer depth counter was not panic-safe, and the bound was silent.
#[test]
fn vf_v6_the_depth_counter_survives_a_panic_and_the_bound_pages() {
    use std::sync::atomic::{AtomicU32, Ordering};

    // (a) A panicking observer used to leave the counter incremented forever; after
    // four of them every economic event in the process was dropped in silence.
    let budget = Arc::new(AtomicU32::new(0));
    let seen = Recorder::default();
    let panicky = Panicky { left: budget.clone() };
    let mut tw = boot_with(
        Vec::new(),
        vec![Arc::new(panicky), Arc::new(seen.clone())],
        Vec::new(),
    );
    let r = run_rig(&mut tw, 5_000);
    let rounds = mush_economy::MAX_OBSERVER_DEPTH + 2;
    budget.store(rounds, Ordering::SeqCst);
    for _ in 0..rounds {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = tw.run(r.johnson, &format!("@econ/pay {}={} 1", r.j_acct, r.r_acct));
        }));
        assert_eq!(
            mush_economy::observer_depth(),
            0,
            "the depth counter did not come back down after a panic"
        );
    }
    assert_eq!(budget.load(Ordering::SeqCst), 0, "the observer never panicked");
    let before = seen.seen.lock().expect("recorder lock").len();
    ok(&mut tw, r.johnson, &format!("@econ/pay {}={} 1", r.j_acct, r.r_acct));
    assert!(
        seen.seen.lock().expect("recorder lock").len() > before,
        "emissions stopped after the panics"
    );

    // (b) The bound itself pages: a dropped event costs a chronicle line, and a cap
    // hit silently in a log is a cap nobody knows was hit.
    let mut tw = boot_with(Vec::new(), vec![Arc::new(Recurser)], Vec::new());
    let r = run_rig(&mut tw, 20_000);
    ok(&mut tw, r.johnson, &format!("@econ/pay {}={} 10", r.j_acct, r.r_acct));
    assert!(
        store(&tw).escalations().any(|e| e.context.contains("depth cap")),
        "the depth cap was hit in silence"
    );
    assert_eq!(store(&tw).escalations().count(), 1, "the cap paged more than once");
}

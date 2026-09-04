//! E4c (08c §6.1 item 3): the **canon evidence bridge**, end to end.
//!
//! This is the one place all three crates meet — `mush-canon` records a sealed scene
//! outcome, `mush-economy`'s attestation observer settles the run it discharges, and
//! `mush-server`'s bridge turns that settlement back into a chronicle line. It lives
//! in `mush-server` because the economy must not depend on canon and this is the
//! composition root that depends on both.
//!
//! What the tests hold to:
//!
//! - **only `Public`** reaches canon — not `Parties`, and emphatically not an
//!   audience list, which is a notice class and not a broadcast;
//! - **existence-level detail only** — no amount, no account, no split, no
//!   description anywhere in the committed line;
//! - **one line per landed transaction** — a replayed transition (the sweep's
//!   `AlreadyPosted` retry, boot finishing a half-done one) publishes nothing new.

use std::collections::BTreeSet;
use std::sync::Arc;

use mush_canon::{CanonDraft, CanonEvent, CanonScope, CanonState};
use mush_core::module::{ModError, Registrar};
use mush_core::{
    ArgShape, Audience, CmdError, CommandSpec, Ctx, GOD, GameModule, ObjId,
};
use mush_economy::{
    AccountId, ContractId, ContractState, ContractStateKind, EconomicEvent, EconomyObserver,
    EconomySnapshot, EconomyStore,
};
use mush_org::{CapabilityId, PrincipalId};
use smol_str::SmolStr;
use mush_proto::OutputEvent;
use mush_server::econ_canon::{CanonEvidenceBridge, KIND_SETTLED};
use mush_testkit::{TestWorld, text_of};

/// A stand-in for `mush-scenes`: `@sealoutcome <kind>=<source>` commits one typed
/// canon record, exactly as a concluded scene does
/// (`mush_scenes::emit_scene_concluded`). It also exposes the bridge directly as
/// `@bridgereplay <contract>=<tx>`, so the replay guard can be driven without
/// contriving a lost document write.
struct SceneStandIn;

impl GameModule for SceneStandIn {
    fn name(&self) -> &'static str {
        "e4cscenes"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.command(
            CommandSpec {
                name: "@sealoutcome".into(),
                shape: ArgShape::EqPair,
                help: "@sealoutcome <kind>=<source>   test: commit one typed canon record".into(),
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                let kind = ctx.args.lhs().trim().to_string();
                let source = ctx.args.rhs().unwrap_or_default().trim().to_string();
                let ic_time = ctx.w.game_now_unix();
                mush_canon::commit(ctx, CanonDraft {
                    kind: SmolStr::new(kind),
                    summary: "A scene concluded.".to_string(),
                    entities: Vec::new(),
                    scope: CanonScope::Restricted,
                    ic_time,
                    source,
                    detail: serde_json::Value::Null,
                })
                .ok_or_else(|| CmdError::msg("canon is not loaded"))
                .map(|_| ())
            }),
        )?;
        reg.command(
            CommandSpec {
                name: "@bridgereplay".into(),
                shape: ArgShape::EqPair,
                help: "@bridgereplay <contract>=<tx>   test: re-deliver a settlement event".into(),
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                let bad = || CmdError::msg("Usage: @bridgereplay <contract>=<tx>");
                let id = ContractId(ctx.args.lhs().trim().parse().map_err(|_| bad())?);
                let tx = mush_economy::TransactionId(
                    ctx.args.rhs().unwrap_or_default().trim().parse().map_err(|_| bad())?,
                );
                CanonEvidenceBridge.on_economic_event(ctx, &EconomicEvent::ContractChanged {
                    id,
                    state: ContractStateKind::Settled,
                    transaction: Some(tx),
                });
                Ok(())
            }),
        )?;
        Ok(())
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

fn boot() -> TestWorld {
    TestWorld::with_modules(vec![
        Box::new(mush_canon::CanonModule::new()),
        Box::new(
            mush_org::OrgModule::new()
                .with_capabilities(vocabulary())
                .with_scope_use(vec![Arc::new(mush_economy::EconomyScopeUse)]),
        ),
        Box::new(
            mush_economy::EconomyModule::with_custody(vec![])
                .with_observers(vec![Arc::new(CanonEvidenceBridge)]),
        ),
        Box::new(SceneStandIn),
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
fn snap(tw: &TestWorld) -> &EconomySnapshot {
    tw.world().module_state::<EconomyStore>().expect("economy store").snapshot()
}
fn state(tw: &TestWorld, id: ContractId) -> ContractState {
    snap(tw).contract(id).expect("contract").state
}
fn canon(tw: &TestWorld) -> Vec<CanonEvent> {
    tw.world().module_state::<CanonState>().expect("canon state").recent()
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

struct Run {
    johnson: ObjId,
    runner: ObjId,
    watcher: ObjId,
    j_acct: AccountId,
    r_acct: AccountId,
}

fn run_rig(tw: &mut TestWorld) -> Run {
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
    ok(tw, GOD, &format!("@econ/mint {faucet}={j_acct} 9999"));
    Run { johnson, runner, watcher, j_acct, r_acct: account_of(tw, runner) }
}

/// A run gated on a sealed scene outcome, at the given visibility, fully accepted.
fn accepted_run(tw: &mut TestWorld, r: &Run, visibility: &str) -> ContractId {
    ok(tw, r.johnson, "@contract/draft Extraction");
    let id = only_contract(tw);
    ok(tw, r.johnson, &format!("@contract/party {id}=#{}", r.runner.0));
    ok(tw, r.johnson, &format!("@contract/pay {id}={} 1234", r.j_acct));
    ok(tw, r.johnson, &format!("@contract/split {id}={} 1234", r.r_acct));
    ok(tw, r.johnson, &format!("@contract/remainder {id}={}", r.j_acct));
    ok(tw, r.johnson, &format!("@contract/condition {id}=attest canon:scene.concluded:scene:5"));
    ok(tw, r.johnson, &format!("@contract/visibility {id}={visibility}"));
    ok(tw, r.johnson, &format!("@contract/offer {id}"));
    ok(tw, r.johnson, &format!("@contract/accept {id}"));
    ok(tw, r.runner, &format!("@contract/accept {id}"));
    assert_eq!(state(tw, id), ContractState::Active, "the run did not come into force");
    id
}

// ======================================================================

#[test]
fn a_sealed_scene_outcome_settles_a_public_run_and_the_settlement_becomes_canon() {
    let mut tw = boot();
    let r = run_rig(&mut tw);
    let id = accepted_run(&mut tw, &r, "public");
    let before = canon(&tw).len();

    let batch = {
        ok(&mut tw, GOD, "@sealoutcome scene.concluded=scene:5");
        tw.drain_write_batch()
    };
    assert!(
        matches!(state(&tw, id), ContractState::Settled { .. }),
        "the sealed outcome did not settle the public run"
    );

    let events = canon(&tw);
    assert_eq!(events.len(), before + 2, "expected the outcome and one settlement line: {events:?}");
    let settled = events
        .iter()
        .find(|e| e.kind == KIND_SETTLED)
        .unwrap_or_else(|| panic!("the settlement did not reach canon: {events:?}"));
    // Existence-level detail only.
    let rendered = format!("{} {} {}", settled.summary, settled.source, settled.detail);
    for leak in ["1234", "escrow", "account", "split", "hold"] {
        assert!(!rendered.contains(leak), "the chronicle line carries `{leak}`: {rendered}");
    }
    // The detail is the contract reference and nothing else — a `Public` contract's
    // id discloses only what `@contract/show` already shows anybody.
    assert_eq!(
        settled.detail,
        serde_json::json!({ "contract": id.0 }),
        "the chronicle line carries more than existence-level detail"
    );
    assert_eq!(settled.scope, CanonScope::Public, "a public contract's line is not public");
    assert!(settled.source.starts_with("economy.tx:"), "the line is not keyed on a transaction: {settled:?}");
    // Canon's own append rides the same batch as the economy's journal chunk.
    assert!(batch.appended_to("canon", "canon"), "canon did not append in this batch");
    assert!(batch.appended_to("economy", "journal"), "the settlement did not journal in this batch");
    let _ = r.watcher;
}

#[test]
fn a_parties_contract_settles_without_reaching_canon() {
    let mut tw = boot();
    let r = run_rig(&mut tw);
    let id = accepted_run(&mut tw, &r, "parties");
    let before = canon(&tw).len();
    ok(&mut tw, GOD, "@sealoutcome scene.concluded=scene:5");
    assert!(matches!(state(&tw, id), ContractState::Settled { .. }), "the run did not settle");
    let events = canon(&tw);
    assert_eq!(events.len(), before + 1, "a `Parties` settlement reached canon: {events:?}");
    assert!(!events.iter().any(|e| e.kind == KIND_SETTLED));
}

#[test]
fn an_audience_is_not_public_and_never_reaches_canon() {
    let mut tw = boot();
    let r = run_rig(&mut tw);
    // Doc 10 T6's eligible defenders are told an operation exists. That is a notice
    // class, and republishing it to the chronicle would invert the whole point.
    let id = accepted_run(&mut tw, &r, &format!("audience #{}", r.watcher.0));
    let before = canon(&tw).len();
    ok(&mut tw, GOD, "@sealoutcome scene.concluded=scene:5");
    assert!(matches!(state(&tw, id), ContractState::Settled { .. }), "the run did not settle");
    let events = canon(&tw);
    assert_eq!(events.len(), before + 1, "an audience contract reached canon: {events:?}");
    assert!(!events.iter().any(|e| e.kind == KIND_SETTLED));
}

#[test]
fn the_bridge_publishes_one_line_per_landed_transaction() {
    let mut tw = boot();
    let r = run_rig(&mut tw);
    let id = accepted_run(&mut tw, &r, "public");
    ok(&mut tw, GOD, "@sealoutcome scene.concluded=scene:5");
    let ContractState::Settled { transaction } = state(&tw, id) else {
        panic!("the run did not settle");
    };
    fn published(tw: &TestWorld) -> usize {
        canon(tw).iter().filter(|e| e.kind == KIND_SETTLED).count()
    }
    assert_eq!(published(&tw), 1);

    // The sweep's `AlreadyPosted` retry and boot's `finished_transition` both produce
    // the same transition again, on the same transaction id.
    ok(&mut tw, GOD, &format!("@bridgereplay {id}={transaction}"));
    ok(&mut tw, GOD, &format!("@bridgereplay {id}={transaction}"));
    assert_eq!(published(&tw), 1, "a replayed settlement was published again");
}

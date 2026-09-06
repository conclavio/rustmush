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
    ArgShape, Audience, CmdError, CommandSpec, Ctx, GOD, GameModule, JobQueueSpec, ObjId, Role,
};
use mush_economy::{
    AccountId, ContractId, ContractState, ContractStateKind, EconomicEvent, EconomyObserver,
    EconomySnapshot, EconomyStore,
};
use mush_org::{CapabilityId, PrincipalId};
use mush_proto::OutputEvent;
use mush_server::econ_canon::{CanonEvidenceBridge, KIND_DELINQUENT, KIND_SETTLED};
use mush_testkit::{TestWorld, text_of};
use smol_str::SmolStr;

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
        reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
        reg.command(
            CommandSpec {
                name: "@sealoutcome".into(),
                shape: ArgShape::EqPair,
                help: "@sealoutcome <kind>=<source> [entity …]   test: commit one canon record"
                    .into(),
                ..CommandSpec::default()
            },
            Arc::new(seal),
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
                    ctx.args
                        .rhs()
                        .unwrap_or_default()
                        .trim()
                        .parse()
                        .map_err(|_| bad())?,
                );
                CanonEvidenceBridge.on_economic_event(
                    ctx,
                    &EconomicEvent::ContractChanged {
                        id,
                        state: ContractStateKind::Settled,
                        transaction: Some(tx),
                    },
                );
                Ok(())
            }),
        )?;
        // E4c verify fold, V1: a canon-committing command that fails **after** the
        // commit. `mush_canon::commit` is terminal by contract, so everything it and
        // its synchronous observers queued must survive the abort.
        reg.command(
            CommandSpec {
                name: "@sealthenfail".into(),
                shape: ArgShape::EqPair,
                help: "@sealthenfail <kind>=<source> [entity …]   test: commit, then fail".into(),
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                seal(ctx)?;
                Err(CmdError::msg("and then the command failed"))
            }),
        )?;
        // E4c verify fold, V2: drive canon's id allocator to its ceiling, so the
        // bridge's own commit fails the way an exhausted allocator makes it fail.
        reg.command(
            CommandSpec {
                name: "@canonalloc".into(),
                shape: ArgShape::Raw,
                help: "@canonalloc <n>   test: set canon's next id".into(),
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                let n: u64 = ctx
                    .args
                    .raw()
                    .trim()
                    .parse()
                    .map_err(|_| CmdError::msg("Usage: @canonalloc <n>"))?;
                let st = ctx
                    .w
                    .module_state_mut::<CanonState>()
                    .ok_or_else(|| CmdError::msg("no canon"))?;
                st.next_canon_id = n;
                Ok(())
            }),
        )?;
        Ok(())
    }
}

/// `<kind>=<source> [entity …]` — one typed canon record.
fn seal(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let kind = ctx.args.lhs().trim().to_string();
    let rhs = ctx.args.rhs().unwrap_or_default().trim().to_string();
    let mut words = rhs.split_whitespace();
    let source = words.next().unwrap_or_default().to_string();
    let entities: Vec<ObjId> = words
        .filter_map(|w| w.trim_start_matches('#').parse().ok())
        .map(ObjId)
        .collect();
    let ic_time = ctx.w.game_now_unix();
    mush_canon::commit(
        ctx,
        CanonDraft {
            kind: SmolStr::new(kind),
            summary: "A scene reached its outcome.".to_string(),
            entities,
            scope: CanonScope::Restricted,
            ic_time,
            source,
            detail: serde_json::Value::Null,
        },
    )
    .ok_or_else(|| CmdError::msg("canon is not loaded"))
    .map(|_| ())
}

/// A payload-job producer, so the E3 loop can be driven end to end (E4c verify
/// fold): `+xspend` files a job whose handler `+job/approve` runs.
struct JobProducer;

impl GameModule for JobProducer {
    fn name(&self) -> &'static str {
        "e4cjobs"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
        reg.job_queue(JobQueueSpec {
            name: "xp".into(),
            title: "XP spends".into(),
            min_role: Role::Wizard,
        })?;
        reg.command(
            CommandSpec {
                name: "+xspend".into(),
                shape: ArgShape::EqPair,
                help: "+xspend <title>=   test: file a payload job".into(),
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                let title = ctx.args.lhs().to_string();
                mush_jobs::open_job(
                    ctx,
                    mush_jobs::OpenJob {
                        queue: "xp".into(),
                        title,
                        opener: ctx.actor,
                        text: None,
                        payload: Some(mush_jobs::JobPayload {
                            handler: "xp.apply".into(),
                            data: serde_json::Value::Null,
                        }),
                        tags: Vec::new(),
                    },
                )
                .map(|_| ())
                .map_err(CmdError::msg)
            }),
        )?;
        Ok(())
    }

    fn on_world_loaded(
        &mut self,
        ctx: &mut mush_core::ModuleCtx<'_>,
        _boot: mush_core::module::ModuleBootData,
    ) -> Result<(), ModError> {
        // Tolerant of a re-registration: the reboot test hands every module its
        // boot data a second time in one process.
        let _ = mush_jobs::register_payload_handler(
            ctx.w,
            "xp.apply",
            Arc::new(|_: &mut Ctx<'_>, _: &mush_jobs::Job| Ok(())),
        );
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
                .with_observers(vec![Arc::new(CanonEvidenceBridge)])
                .with_facts(vec![Arc::new(mush_jobs::JobsFactProvider)])
                // E4c verify fold (E1): `scene.outcome` stands in for the record
                // E6b will seal under facilitator authority. `scene.concluded` is
                // deliberately absent — a lifecycle record any participant can emit
                // is not an outcome, and a contract gated on one is a contract its
                // payee can settle alone.
                .with_canon_outcome_kinds(vec![SmolStr::new_static("scene.outcome")]),
        ),
        Box::new(mush_jobs::JobsModule::new()),
        Box::new(JobProducer),
        Box::new(SceneStandIn),
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
fn snap(tw: &TestWorld) -> &EconomySnapshot {
    tw.world()
        .module_state::<EconomyStore>()
        .expect("economy store")
        .snapshot()
}
fn state(tw: &TestWorld, id: ContractId) -> ContractState {
    snap(tw).contract(id).expect("contract").state
}
fn canon(tw: &TestWorld) -> Vec<CanonEvent> {
    tw.world()
        .module_state::<CanonState>()
        .expect("canon state")
        .recent()
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
    Run {
        johnson,
        runner,
        watcher,
        j_acct,
        r_acct: account_of(tw, runner),
    }
}

/// A run gated on a sealed scene outcome, at the given visibility, fully accepted.
fn accepted_run(tw: &mut TestWorld, r: &Run, visibility: &str) -> ContractId {
    ok(tw, r.johnson, "@contract/draft Extraction");
    let id = only_contract(tw);
    ok(
        tw,
        r.johnson,
        &format!("@contract/party {id}=#{}", r.runner.0),
    );
    ok(
        tw,
        r.johnson,
        &format!("@contract/pay {id}={} 1234", r.j_acct),
    );
    ok(
        tw,
        r.johnson,
        &format!("@contract/split {id}={} 1234", r.r_acct),
    );
    ok(
        tw,
        r.johnson,
        &format!("@contract/remainder {id}={}", r.j_acct),
    );
    ok(
        tw,
        r.johnson,
        &format!("@contract/condition {id}=attest canon:scene.outcome:scene:5"),
    );
    ok(
        tw,
        r.johnson,
        &format!("@contract/visibility {id}={visibility}"),
    );
    ok(tw, r.johnson, &format!("@contract/offer {id}"));
    ok(tw, r.johnson, &format!("@contract/accept {id}"));
    ok(tw, r.runner, &format!("@contract/accept {id}"));
    assert_eq!(
        state(tw, id),
        ContractState::Active,
        "the run did not come into force"
    );
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
        ok(
            &mut tw,
            GOD,
            &format!("@sealoutcome scene.outcome=scene:5 #{}", r.runner.0),
        );
        tw.drain_write_batch()
    };
    assert!(
        matches!(state(&tw, id), ContractState::Settled { .. }),
        "the sealed outcome did not settle the public run"
    );

    let events = canon(&tw);
    assert_eq!(
        events.len(),
        before + 2,
        "expected the outcome and one settlement line: {events:?}"
    );
    let settled = events
        .iter()
        .find(|e| e.kind == KIND_SETTLED)
        .unwrap_or_else(|| panic!("the settlement did not reach canon: {events:?}"));
    // Existence-level detail only.
    let rendered = format!("{} {} {}", settled.summary, settled.source, settled.detail);
    for leak in ["1234", "escrow", "account", "split", "hold"] {
        assert!(
            !rendered.contains(leak),
            "the chronicle line carries `{leak}`: {rendered}"
        );
    }
    // The detail is the contract reference and nothing else — a `Public` contract's
    // id discloses only what `@contract/show` already shows anybody.
    assert_eq!(
        settled.detail,
        serde_json::json!({ "contract": id.0 }),
        "the chronicle line carries more than existence-level detail"
    );
    assert_eq!(
        settled.scope,
        CanonScope::Public,
        "a public contract's line is not public"
    );
    assert!(
        settled.source.starts_with("economy.tx:"),
        "the line is not keyed on a transaction: {settled:?}"
    );
    // Canon's own append rides the same batch as the economy's journal chunk.
    assert!(
        batch.appended_to("canon", "canon"),
        "canon did not append in this batch"
    );
    assert!(
        batch.appended_to("economy", "journal"),
        "the settlement did not journal in this batch"
    );
    let _ = r.watcher;
}

#[test]
fn a_parties_contract_settles_without_reaching_canon() {
    let mut tw = boot();
    let r = run_rig(&mut tw);
    let id = accepted_run(&mut tw, &r, "parties");
    let before = canon(&tw).len();
    ok(
        &mut tw,
        GOD,
        &format!("@sealoutcome scene.outcome=scene:5 #{}", r.runner.0),
    );
    assert!(
        matches!(state(&tw, id), ContractState::Settled { .. }),
        "the run did not settle"
    );
    let events = canon(&tw);
    assert_eq!(
        events.len(),
        before + 1,
        "a `Parties` settlement reached canon: {events:?}"
    );
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
    ok(
        &mut tw,
        GOD,
        &format!("@sealoutcome scene.outcome=scene:5 #{}", r.runner.0),
    );
    assert!(
        matches!(state(&tw, id), ContractState::Settled { .. }),
        "the run did not settle"
    );
    let events = canon(&tw);
    assert_eq!(
        events.len(),
        before + 1,
        "an audience contract reached canon: {events:?}"
    );
    assert!(!events.iter().any(|e| e.kind == KIND_SETTLED));
}

#[test]
fn the_bridge_publishes_one_line_per_landed_transaction() {
    let mut tw = boot();
    let r = run_rig(&mut tw);
    let id = accepted_run(&mut tw, &r, "public");
    ok(
        &mut tw,
        GOD,
        &format!("@sealoutcome scene.outcome=scene:5 #{}", r.runner.0),
    );
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
    assert_eq!(
        published(&tw),
        1,
        "a replayed settlement was published again"
    );
}

// ======================================================================
// ---- E4c verify fold ----
// ======================================================================

/// Both namespaces plus canon, folded: a reboot is one event in the real server.
type Kv = std::collections::HashMap<SmolStr, Vec<(SmolStr, serde_json::Value)>>;

fn fold_kv(tw: &mut TestWorld, acc: &mut Kv, skip: &[(&str, String)]) {
    for op in tw.take_module_ops() {
        if skip.iter().any(|(ns, key)| {
            *ns == op.ns.as_str()
                && matches!(&op.op, mush_core::ModuleWrite::JsonSet { key: k, .. } if k.as_str() == key)
        }) {
            continue;
        }
        let rows = acc.entry(op.ns.clone()).or_default();
        match op.op {
            mush_core::ModuleWrite::JsonSet { key, value } => {
                rows.retain(|(k, _)| *k != key);
                rows.push((key, value));
            }
            mush_core::ModuleWrite::Del { key } => rows.retain(|(k, _)| *k != key),
            mush_core::ModuleWrite::StreamAppend { .. } => {}
        }
    }
}

fn reboot(tw: &mut TestWorld, kv: &Kv) {
    let boot: std::collections::HashMap<SmolStr, mush_core::module::ModuleBootData> = kv
        .iter()
        .map(|(ns, rows)| {
            (
                ns.clone(),
                mush_core::module::ModuleBootData {
                    kv: rows.clone(),
                    streams: std::collections::HashMap::new(),
                    ..Default::default()
                },
            )
        })
        .collect();
    tw.modules_loaded(boot);
}

fn sweep_at(tw: &mut TestWorld, t: i64) {
    tw.set_now(t);
    let _ = tw.fire_due(t.saturating_mul(1000));
}

fn published(tw: &TestWorld, kind: &str) -> usize {
    canon(tw).iter().filter(|e| e.kind == kind).count()
}

fn marker(tw: &TestWorld, id: ContractId) -> Option<SmolStr> {
    snap(tw)
        .contract(id)
        .expect("contract")
        .canon_published
        .clone()
}

/// D7. A delinquency line named every party of the contract the obligation came
/// from. "Who is behind on their rent" is a fact about the debtor.
#[test]
fn vf_d7_a_delinquency_names_the_debtor_and_nobody_else() {
    let mut tw = boot();
    let r = run_rig(&mut tw);
    // A public lease: the Johnson funds it, the runner rents, and the watcher is a
    // third party on the same agreement who owes nothing at all.
    let w_acct = account_of(&tw, r.watcher);
    ok(&mut tw, r.johnson, "@contract/draft Lease");
    let id = only_contract(&tw);
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/party {id}=#{}", r.runner.0),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/party {id}=#{}", r.watcher.0),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/pay {id}={} 100", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/split {id}={} 100", r.r_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/remainder {id}={}", r.j_acct),
    );
    // The tenant's wallet is empty, so the first period is missed.
    ok(
        &mut tw,
        r.johnson,
        &format!(
            "@contract/deliver {id}=obligation {} {} 50 every=600 grace=0",
            r.r_acct, r.j_acct
        ),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/condition {id}=attest party:#{}", r.johnson.0),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/visibility {id}=public"),
    );
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    for who in [r.johnson, r.runner, r.watcher] {
        ok(&mut tw, who, &format!("@contract/accept {id}"));
    }
    let _ = w_acct;

    sweep_at(&mut tw, 5_000);
    let lines: Vec<_> = canon(&tw)
        .into_iter()
        .filter(|e| e.kind == KIND_DELINQUENT)
        .collect();
    assert_eq!(
        lines.len(),
        1,
        "the delinquency was not chronicled once: {lines:?}"
    );
    assert_eq!(
        lines[0].entities,
        vec![r.runner],
        "the delinquency named somebody other than the debtor"
    );
}

/// V1. A canon-committing command that returns `Err` after the commit used to drop
/// the whole batch while RAM kept the settlement: canon recorded, an escrow
/// released, and neither on disk.
#[test]
fn vf_v1_a_terminal_commit_survives_a_failing_command() {
    let mut tw = boot();
    let r = run_rig(&mut tw);
    let id = accepted_run(&mut tw, &r, "public");
    let _ = tw.drain_write_batch();

    // The command seals the outcome — which settles the run through the economy's
    // observer — and *then* fails.
    let ev = tw.run(
        GOD,
        &format!("@sealthenfail scene.outcome=scene:5 #{}", r.runner.0),
    );
    assert!(
        has_error(&ev),
        "the test command was supposed to fail: {}",
        texts(&ev)
    );
    let batch = tw.drain_write_batch();

    assert!(
        matches!(state(&tw, id), ContractState::Settled { .. }),
        "the outcome did not settle the run"
    );
    assert!(
        batch.appended_to("canon", "canon"),
        "canon's own append was truncated"
    );
    assert!(
        batch.appended_to("economy", "journal"),
        "the settlement's journal chunk was truncated"
    );
    let doc = format!("contract:{id}");
    assert!(
        batch.module_ops.iter().any(|o| o.ns == "economy"
            && matches!(&o.op, mush_core::ModuleWrite::JsonSet { key, .. } if key.as_str() == doc)),
        "the settled contract document was truncated"
    );
}

/// V2. Publication was best-effort: a `mush_canon::commit` that returned `None`
/// lost the line behind a `let _ =`, and nothing ever tried again.
#[test]
fn vf_v2_a_failed_canon_commit_is_retried_by_the_next_sweep_and_chronicled_once() {
    let mut tw = boot();
    let r = run_rig(&mut tw);
    let id = accepted_run(&mut tw, &r, "public");
    tw.set_now(1_000);
    // Leave canon exactly one id: the sealed outcome takes it, and the bridge's own
    // commit finds the allocator at its ceiling.
    ok(&mut tw, GOD, &format!("@canonalloc {}", u64::MAX - 1));
    ok(
        &mut tw,
        GOD,
        &format!("@sealoutcome scene.outcome=scene:5 #{}", r.runner.0),
    );
    assert!(
        matches!(state(&tw, id), ContractState::Settled { .. }),
        "the run did not settle"
    );
    assert_eq!(
        published(&tw, KIND_SETTLED),
        0,
        "the commit was supposed to fail"
    );
    assert_eq!(
        marker(&tw, id),
        None,
        "a failed commit still marked the contract published"
    );

    // A sweep later, with the allocator healthy again, the settlement is announced
    // again and the bridge gets its line.
    ok(&mut tw, GOD, "@canonalloc 500");
    sweep_at(&mut tw, 1_100);
    assert_eq!(published(&tw, KIND_SETTLED), 1, "the retry never happened");
    assert!(
        marker(&tw, id).is_some(),
        "the retry did not mark the contract published"
    );
    // …and it stays one line, however many fires follow.
    for t in [1_200, 1_300, 1_400] {
        sweep_at(&mut tw, t);
    }
    assert_eq!(
        published(&tw, KIND_SETTLED),
        1,
        "the sweep republished a chronicled settlement"
    );
}

/// V3. Boot's `finished_transition` moves a contract to `Settled` by writing the
/// document directly, so no event is ever emitted — a settlement recovered at boot
/// was never chronicled at all.
#[test]
fn vf_v3_a_boot_healed_public_settlement_is_chronicled_exactly_once() {
    let mut tw = boot();
    let r = run_rig(&mut tw);
    let id = accepted_run(&mut tw, &r, "public");
    tw.set_now(1_000);
    let mut kv: Kv = Kv::new();
    fold_kv(&mut tw, &mut kv, &[]);

    // Settle it with the bridge unable to publish (so the pre-reboot world carries
    // no line either), and lose exactly the write that records the new state — the
    // shape a crash between the post and the document write leaves behind.
    ok(&mut tw, GOD, &format!("@canonalloc {}", u64::MAX - 1));
    ok(
        &mut tw,
        GOD,
        &format!("@sealoutcome scene.outcome=scene:5 #{}", r.runner.0),
    );
    assert!(
        matches!(state(&tw, id), ContractState::Settled { .. }),
        "the run did not settle"
    );
    assert_eq!(published(&tw, KIND_SETTLED), 0);
    fold_kv(&mut tw, &mut kv, &[("economy", format!("contract:{id}"))]);

    reboot(&mut tw, &kv);
    ok(&mut tw, GOD, "@canonalloc 500");
    // Boot read the contract as live, found its escrow gone from the journal, and
    // finished the transition from the idempotency index — silently.
    assert!(
        matches!(state(&tw, id), ContractState::Settled { .. }),
        "boot did not heal the lost transition"
    );
    assert_eq!(
        marker(&tw, id),
        None,
        "a boot-healed settlement claimed a chronicle line"
    );
    assert_eq!(
        published(&tw, KIND_SETTLED),
        0,
        "canon rebooted with a line it should not have"
    );

    sweep_at(&mut tw, 1_200);
    assert_eq!(
        published(&tw, KIND_SETTLED),
        1,
        "the boot-healed settlement was never chronicled"
    );
    sweep_at(&mut tw, 1_400);
    assert_eq!(published(&tw, KIND_SETTLED), 1, "it was chronicled twice");
}

/// V4. The delinquency draft was keyed on the obligation alone, so the *second*
/// time a tenant fell behind on the same lease the guard read its own first line
/// and published nothing.
#[test]
fn vf_v4_a_second_delinquency_is_a_second_line() {
    let mut tw = boot();
    let r = run_rig(&mut tw);
    ok(&mut tw, r.johnson, "@contract/draft Lease");
    let id = only_contract(&tw);
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/party {id}=#{}", r.runner.0),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/pay {id}={} 100", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/split {id}={} 100", r.r_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/remainder {id}={}", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!(
            "@contract/deliver {id}=obligation {} {} 50 every=600 grace=0",
            r.r_acct, r.j_acct
        ),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/condition {id}=attest party:#{}", r.johnson.0),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/visibility {id}=public"),
    );
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
    ok(&mut tw, r.runner, &format!("@contract/accept {id}"));

    sweep_at(&mut tw, 5_000);
    assert_eq!(
        published(&tw, KIND_DELINQUENT),
        1,
        "the first delinquency was not chronicled"
    );
    let oid = snap(&tw).obligations().next().expect("obligation").id;

    // The tenant catches up, the lease is reinstated, and they fall behind again.
    ok(&mut tw, GOD, &format!("@obligation/reinstate {oid}"));
    sweep_at(&mut tw, 20_000);
    assert_eq!(
        published(&tw, KIND_DELINQUENT),
        2,
        "the second delinquency read the first one's line and said nothing: {:?}",
        canon(&tw)
    );
}

/// V5. The replay guard read canon's RAM tail, which rolls; the durable marker on
/// the contract is what refuses a replay now, whatever key it carries.
#[test]
fn vf_v5_the_durable_marker_and_not_the_tail_refuses_a_replay() {
    let mut tw = boot();
    let r = run_rig(&mut tw);
    let id = accepted_run(&mut tw, &r, "public");
    ok(
        &mut tw,
        GOD,
        &format!("@sealoutcome scene.outcome=scene:5 #{}", r.runner.0),
    );
    let ContractState::Settled { transaction } = state(&tw, id) else {
        panic!("did not settle")
    };
    assert_eq!(published(&tw, KIND_SETTLED), 1);
    let published_ref = marker(&tw, id).expect("the settlement was not marked published");
    assert!(
        published_ref.starts_with("canon:"),
        "the marker is not a canon reference"
    );

    // The same transition again — the sweep's `AlreadyPosted` retry.
    ok(&mut tw, GOD, &format!("@bridgereplay {id}={transaction}"));
    assert_eq!(
        published(&tw, KIND_SETTLED),
        1,
        "a replay was published again"
    );
    // …and a replay carrying a DIFFERENT transaction id, which the tail-keyed guard
    // would have waved through as a new settlement.
    ok(
        &mut tw,
        GOD,
        &format!("@bridgereplay {id}={}", transaction.0 + 99),
    );
    assert_eq!(
        published(&tw, KIND_SETTLED),
        1,
        "the guard was still the source key rather than the contract's own marker"
    );
    assert_eq!(
        marker(&tw, id),
        Some(published_ref),
        "the marker moved to a second line"
    );
}

/// E3, through the real loop: a payload job's **status** is not the fact a contract
/// settles on, and `+job/status` cannot manufacture one.
#[test]
fn vf_e3_a_status_flip_settles_nothing_and_an_approval_does() {
    let mut tw = boot();
    let r = run_rig(&mut tw);
    ok(&mut tw, r.johnson, "@contract/draft Courier");
    let id = only_contract(&tw);
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/party {id}=#{}", r.runner.0),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/pay {id}={} 400", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/split {id}={} 400", r.r_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/remainder {id}={}", r.j_acct),
    );
    ok(
        &mut tw,
        r.johnson,
        &format!("@contract/condition {id}=fact jobs resolved 1"),
    );
    ok(&mut tw, r.johnson, &format!("@contract/offer {id}"));
    ok(&mut tw, r.johnson, &format!("@contract/accept {id}"));
    ok(&mut tw, r.runner, &format!("@contract/accept {id}"));

    ok(&mut tw, r.runner, "+xspend Courier run=");
    // Staff flipping the status settles nothing: it is refused outright, and the
    // fact stays false either way.
    let _ = tw.run(GOD, "+job/status 1=resolved");
    sweep_at(&mut tw, 5_000);
    assert_eq!(
        state(&tw, id),
        ContractState::Active,
        "a status flip released the escrow"
    );

    // The approval runs the payload, writes the applied marker, and the fact is a
    // fact.
    ok(&mut tw, GOD, "+job/approve 1=done");
    sweep_at(&mut tw, 5_100);
    assert!(
        matches!(state(&tw, id), ContractState::Settled { .. }),
        "an applied payload did not settle the run"
    );
}

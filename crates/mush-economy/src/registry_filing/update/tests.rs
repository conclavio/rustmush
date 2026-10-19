//! Actual registered owner preparation and native fee posting. The Roll fixture
//! supplies only action definitions; this is not a Registry/certificate fixture.
use super::*;
use mush_core::action::{
    ActionError, ActionErrorCode, ActionInvocation, ActionInvocationDraft, ActionOfferId,
    ActionProvider, AuthoritativeActionOutput, CanonicalHash, IdempotencyKey as ActionKey,
    IdempotencyScope, PerceivedRef, PreparedAuthoritativeAction, PreparedCausalRef,
    PrincipalId as ActionPrincipal, RequestUid, ScopedIdempotencyKey,
};
use mush_core::authoritative::{AuthoritativeBundle, AuthoritativeBundleDraft};
use mush_core::participant::ParticipantHandle;
use mush_core::{
    ActionTxn, CanonicalId, DefinitionDescriptor, DefinitionId, DefinitionKind,
    DefinitionRegistryBuilder, DefinitionValidationContext, GOD, GameModule, ModError,
    ModuleBootData, ModuleOp, ModuleWrite, ObjKind, PrepareCtx, Registrar, RevisionSet, RulesValue,
};
use mush_testkit::{TestWorld, fold_module_ops_into};
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
type Boot = HashMap<SmolStr, ModuleBootData>;
type PreparedAction = (
    mush_core::composed::PreparedComposedTxn,
    PreparedAuthoritativeAction,
    ActionInvocation,
);

#[derive(Clone)]
struct Rules {
    declarations: Vec<DefinitionDescriptor>,
    initial: DefinitionRef,
    update: DefinitionRef,
    wrong: DefinitionRef,
    outcome: DefinitionRef,
}
fn descriptor(kind: &str, id: &str) -> Result<DefinitionDescriptor> {
    Ok(DefinitionDescriptor::new(
        DefinitionKind::new(kind)?,
        DefinitionId::new(id)?,
        1,
        json!({"fixture":"fee-only-update"}),
    )?)
}
fn rules() -> Result<Rules> {
    let mut b = DefinitionRegistryBuilder::new();
    let owner = OwnerId::new("roll")?;
    let mut declarations = Vec::new();
    let mut add = |kind, id| -> Result<DefinitionRef> {
        let d = descriptor(kind, id)?;
        let r = b.register(owner.clone(), d.clone())?;
        declarations.push(d);
        Ok(r)
    };
    let initial = add("action", "initial_filing")?;
    let update = add("action", ACTION_ID)?;
    let wrong = add("action", "wrong_update")?;
    let outcome = add("outcome", "fee_posted")?;
    Ok(Rules {
        declarations,
        initial,
        update,
        wrong,
        outcome,
    })
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    charter: ObjId,
    payer: AccountId,
    title_revision: u64,
}
struct Provider {
    handle: ParticipantHandle<InitialFilingParticipant>,
    initial: bool,
    outcome: DefinitionRef,
}
fn error() -> ActionError {
    ActionError::new(
        ActionErrorCode::ProviderRejected,
        false,
        "fee update fixture refused",
    )
    .expect("static error")
}
impl ActionProvider for Provider {
    fn validate_invocation(&self, _: &ActionInvocation) -> std::result::Result<(), ActionError> {
        Ok(())
    }
    fn prepare_authoritative_with_context(
        &self,
        inv: &ActionInvocation,
        _: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> std::result::Result<AuthoritativeActionOutput, ActionError> {
        let r: Input = serde_json::from_value(inv.parameters().clone()).map_err(|_| error())?;
        let request = if self.initial {
            EconomyRequest::InitialFiling(RegistryInitialFiling {
                shell: r.charter,
                payer: r.payer,
                owner: PrincipalId::Object(inv.actor()),
                scope: None,
            })
        } else {
            EconomyRequest::CompatibleUpdate(RegistryCompatibleUpdate {
                charter: r.charter,
                payer: r.payer,
                title_revision: r.title_revision,
            })
        };
        let output = ctx
            .prepare_owner(&self.handle, request)
            .map_err(|_| error())?;
        let value = if self.initial {
            json!({"posted":true})
        } else {
            serde_json::to_value(output.compatible_update().map_err(|_| error())?.subject())
                .map_err(|_| error())?
        };
        let result = RulesValue::new(self.outcome.clone(), value).map_err(|_| error())?;
        Ok(AuthoritativeActionOutput::new(result).with_definitions(vec![self.outcome.clone()]))
    }
}
impl GameModule for Rules {
    fn name(&self) -> &'static str {
        "roll"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> std::result::Result<(), ModError> {
        for kind in ["action", "outcome"] {
            let expected: Vec<_> = self
                .declarations
                .iter()
                .filter(|d| d.kind().as_str() == kind)
                .cloned()
                .collect();
            reg.definition_validator(
                DefinitionKind::new(kind).map_err(|e| ModError::msg(e.to_string()))?,
                move |ctx: &DefinitionValidationContext<'_>| {
                    let e = ctx.definition();
                    if expected.iter().any(|d| {
                        d.id() == &e.reference().key().id
                            && d.payload() == e.payload()
                            && d.references() == e.references()
                    }) {
                        Ok(())
                    } else {
                        Err("fixture definition drift".into())
                    }
                },
            )?;
        }
        for d in &self.declarations {
            reg.definition(d.clone())?;
        }
        let initial =
            reg.request_owner_participant::<InitialFilingParticipant>(self.initial.clone())?;
        let update =
            reg.request_owner_participant::<InitialFilingParticipant>(self.update.clone())?;
        for (action, handle, is_initial) in [
            (self.initial.clone(), initial, true),
            (self.update.clone(), update.clone(), false),
            (self.wrong.clone(), update, false),
        ] {
            reg.action_provider(
                action,
                Provider {
                    handle,
                    initial: is_initial,
                    outcome: self.outcome.clone(),
                },
            )?;
        }
        Ok(())
    }
}
fn policy(fee: i64) -> InitialFilingPolicy {
    InitialFilingPolicy::new(
        SystemPrincipalId::new("roll.registry").expect("issuer"),
        AccountId(2),
        Amount::from_minor(fee),
        CatalogRef {
            ruleset: "roll".into(),
            code: "mind_charter".into(),
        },
    )
    .expect("policy")
}
fn modules(r: &Rules, fee: i64) -> Vec<Box<dyn GameModule>> {
    vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(r.clone()),
        Box::new(
            crate::EconomyModule::new()
                .with_title_issuers(vec![
                    SystemPrincipalId::new("roll.registry").expect("issuer"),
                ])
                .with_initial_filing(policy(fee), r.initial.clone())
                .with_compatible_mind_updates(r.update.clone()),
        ),
    ]
}
struct Fixture {
    world: TestWorld,
    rules: Rules,
    actor: ObjId,
    shell: ObjId,
    boot: Boot,
}
impl Fixture {
    fn empty() -> Result<Self> {
        let rules = rules()?;
        let mut world = TestWorld::with_modules(modules(&rules, 100));
        let actor = world.spawn_player("Updater");
        let shell =
            world
                .world_mut()
                .create(ObjKind::Thing, "Reserved update charter", GOD, None)?;
        Ok(Self {
            world,
            rules,
            actor,
            shell,
            boot: Boot::new(),
        })
    }
    fn new() -> Result<Self> {
        let mut f = Self::empty()?;
        for command in [
            "@instrument/register cr=0".to_owned(),
            "@account/system faucet=cr".to_owned(),
            format!("@account/open #{}=cr", f.actor.0),
            "@account/system roll.registry=cr".to_owned(),
            "@econ/mint 0=1 10000".to_owned(),
        ] {
            let events = f.world.run(GOD, &command);
            if events.iter().any(|(_,e)|matches!(e,mush_proto::OutputEvent::Sys(t) if t.spans().iter().any(|s|s.tag.as_deref()==Some("error")))) {return Err("actual economic fixture setup refused".into());}
        }
        fold_module_ops_into(&mut f.boot, f.world.take_module_ops());
        let _ = f.world.world_mut().take_dirty();
        let action = f.rules.initial.clone();
        let p = f.prepare(&action, f.actor, 1)?;
        f.publish(action, 1, p)?;
        Ok(f)
    }
    fn state(&self) -> &EconomyStore {
        self.world
            .world()
            .module_state::<EconomyStore>()
            .expect("actual economy")
    }
    fn state_mut(&mut self) -> &mut EconomyStore {
        self.world
            .world_mut()
            .module_state_mut::<EconomyStore>()
            .expect("actual economy")
    }
    fn prepare(&mut self, action: &DefinitionRef, actor: ObjId, n: u8) -> Result<PreparedAction> {
        let token = token(n);
        let request = RequestUid::from_bytes([n; 16]);
        let principal = ActionPrincipal::Object { id: actor };
        let inv = ActionInvocation::new(ActionInvocationDraft {
            request,
            principal: principal.clone(),
            actor,
            offered_action: ActionOfferId(CanonicalHash::from_bytes([2; 32])),
            subject: PerceivedRef::SelfActor,
            targets: vec![],
            parameters: json!({"charter":self.shell,"payer":AccountId(1),"title_revision":self.state().snapshot.asset(self.shell).map_or(0,|a|a.revision)}),
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal,
                    owner: OwnerId::new("roll")?,
                    namespace: CanonicalId::new("update_fixture")?,
                },
                1,
                ActionKey::Request(request),
            ),
        })?;
        let (p, o) = self
            .world
            .engine_mut()
            .prepare_registered_composed_authoritative_action(action, &inv, token)?;
        Ok((p, o, inv))
    }
    fn update(&mut self, n: u8) -> Result<PreparedAction> {
        let action = self.rules.update.clone();
        self.prepare(&action, self.actor, n)
    }
    fn stage(
        &mut self,
        action: DefinitionRef,
        n: u8,
        p: PreparedAction,
    ) -> Result<Vec<ReplayWrite>> {
        let (p, o, inv) = p;
        let writes = p.authoritative_replay_writes();
        let (outcome, resolution, mut definitions, mut witnesses, mut outbox, events) =
            o.into_parts();
        definitions.push(action.clone());
        definitions.push(outcome.schema().clone());
        definitions.sort();
        definitions.dedup();
        if witnesses.is_empty() {
            witnesses.push(inv.principal().clone());
        }
        witnesses.sort();
        witnesses.dedup();
        outbox.sort_by_key(|o| o.ordinal());
        let bundle = AuthoritativeBundle::new(AuthoritativeBundleDraft {
            token: token(n),
            request: inv.request(),
            idempotency: inv.idempotency().clone(),
            ingress_hash: CanonicalHash::from_bytes([3; 32]),
            request_hash: inv.request_hash(),
            plan_hash: CanonicalHash::from_bytes([4; 32]),
            action,
            provider: p.owner().clone(),
            cause: PreparedCausalRef::new(inv.request(), None, 0)?,
            definitions,
            writes: writes.clone(),
            outcome,
            resolution: resolution.map(|r| r.bind(token(n))),
            semantic_events: events,
            eligible_witnesses: witnesses,
            durable_outbox: outbox,
        })?;
        let guards = self
            .world
            .registry()
            .server_guard_set_builder(RevisionSet::default())?
            .finish()?;
        self.world
            .engine_mut()
            .stage_authoritative_action(p, guards, bundle)?;
        Ok(writes)
    }
    fn take_install(&mut self) -> Result {
        let held = self
            .world
            .engine_mut()
            .take_staged_action()?
            .ok_or("actual staged action")?;
        let (_, installation) = held.into_parts();
        self.world
            .engine_mut()
            .install_composed_action(installation)?;
        Ok(())
    }
    fn publish(
        &mut self,
        action: DefinitionRef,
        n: u8,
        p: PreparedAction,
    ) -> Result<Vec<ReplayWrite>> {
        let writes = self.stage(action, n, p)?;
        self.take_install()?;
        let mut ops = Vec::new();
        for w in &writes {
            ops.push(match w {
                ReplayWrite::ModuleJsonSet {
                    namespace,
                    key,
                    value,
                } => ModuleOp {
                    ns: namespace.as_str().into(),
                    op: ModuleWrite::JsonSet {
                        key: key.as_str().into(),
                        value: value.clone(),
                    },
                },
                ReplayWrite::LedgerChunkSet { write } => ModuleOp {
                    ns: write.namespace().as_str().into(),
                    op: ModuleWrite::JsonSet {
                        key: write.key().as_str().into(),
                        value: serde_json::to_value(write.chunk())?,
                    },
                },
                _ => return Err("unexpected update replay write".into()),
            });
        }
        fold_module_ops_into(&mut self.boot, ops);
        Ok(writes)
    }
}
fn token(n: u8) -> CommitToken {
    CommitToken::from_bytes([n; 16])
}

#[test]
fn actual_registered_update_posts_one_fee_preserves_birth_title_and_replays_exactly() -> Result {
    let mut f = Fixture::new()?;
    let title = f.state().snapshot.asset(f.shell).ok_or("title")?.clone();
    let filing = serde_json::to_value(f.state().filing_receipt_audit())?;
    let before = f.state().snapshot.clone();
    let p = f.update(2)?;
    assert_eq!(f.state().snapshot, before);
    let writes = f.stage(f.rules.update.clone(), 2, p)?;
    assert_eq!(f.state().snapshot, before);
    f.take_install()?;
    assert_eq!(f.state().snapshot.asset(f.shell), Some(&title));
    assert_eq!(
        serde_json::to_value(f.state().filing_receipt_audit())?,
        filing
    );
    assert_eq!(
        f.state()
            .snapshot
            .account(AccountId(1))
            .ok_or("payer")?
            .posted_balance,
        Amount::from_minor(9800)
    );
    assert_eq!(
        f.state()
            .snapshot
            .account(AccountId(2))
            .ok_or("sink")?
            .posted_balance,
        Amount::from_minor(200)
    );
    assert_eq!(writes.len(), 5);
    assert!(!writes.iter().any(
        |w| matches!(w,ReplayWrite::ModuleJsonSet{key,..} if key.as_str().starts_with("asset:"))
    ));
    let subject = f
        .state()
        .update_receipt_audit()
        .receipts()?
        .next()
        .ok_or("receipt")?
        .clone();
    f.state().update_receipt_audit().verify(&subject)?;
    let actual = f
        .state()
        .transaction(subject.transaction)
        .ok_or("native transaction")?;
    assert_eq!(receipt::subject_of(actual)?, subject);
    let mut replay = UpdateReceiptAudit::default();
    let mut tx: Vec<_> = f.state().transactions.values().collect();
    tx.sort_by_key(|t| t.journal_seq);
    for t in tx {
        replay.observe(t);
    }
    assert!(replay.verify(&subject).is_err());
    replay.finish_replay()?;
    replay.verify(&subject)?;
    assert_eq!(replay, *f.state().update_receipt_audit());
    assert!(f.update(2).is_err());
    Ok(())
}

#[test]
fn update_rechecks_title_liveness_and_account_authority_at_stage_and_take() -> Result {
    for at_take in [false, true] {
        let mut f = Fixture::new()?;
        let p = f.update(2)?;
        let action = f.rules.update.clone();
        if at_take {
            f.stage(action.clone(), 2, p)?;
            let shell = f.shell;
            f.state_mut()
                .snapshot
                .asset_mut(shell)
                .ok_or("title")?
                .frozen = true;
            let before = f.state().snapshot.clone();
            assert!(f.world.engine_mut().take_staged_action().is_err());
            assert_eq!(f.state().snapshot, before);
        } else {
            let shell = f.shell;
            f.state_mut()
                .snapshot
                .asset_mut(shell)
                .ok_or("title")?
                .revision += 1;
            let before = f.state().snapshot.clone();
            assert!(f.stage(action, 2, p).is_err());
            assert_eq!(f.state().snapshot, before);
        }
        assert_eq!(f.state().update_receipt_audit().receipts()?.len(), 0);
    }
    let mut f = Fixture::new()?;
    let other = f.world.spawn_player("Foreign updater");
    let action = f.rules.update.clone();
    assert!(f.prepare(&action, other, 2).is_err());
    let wrong = f.rules.wrong.clone();
    let actor = f.actor;
    assert!(f.prepare(&wrong, actor, 2).is_err());
    f.state_mut()
        .snapshot
        .account_mut(AccountId(1))
        .ok_or("payer")?
        .owner = PrincipalId::Object(other);
    assert!(f.update(2).is_err());
    f.state_mut()
        .snapshot
        .account_mut(AccountId(1))
        .ok_or("payer")?
        .owner = PrincipalId::Object(actor);
    f.world
        .world_mut()
        .get_mut(actor)
        .ok_or("actor")?
        .flags
        .insert(mush_core::Flags::GOING);
    assert!(f.update(2).is_err());
    Ok(())
}

#[test]
fn real_cold_boot_requires_paired_history_and_preserves_initial_receipt() -> Result {
    let mut f = Fixture::new()?;
    let p = f.update(2)?;
    f.publish(f.rules.update.clone(), 2, p)?;
    let subject = f
        .state()
        .update_receipt_audit()
        .receipts()?
        .next()
        .ok_or("receipt")?
        .clone();
    let mut cold = Fixture::empty()?;
    assert_eq!(cold.shell, f.shell);
    cold.world.try_modules_loaded(f.boot.clone())?;
    cold.state().update_receipt_audit().verify(&subject)?;
    assert_eq!(cold.state().filing_receipt_audit().receipts()?.len(), 1);
    assert_eq!(
        cold.state().snapshot.asset(f.shell),
        f.state().snapshot.asset(f.shell)
    );
    let mut disabled = TestWorld::with_modules(vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(crate::EconomyModule::new().with_title_issuers(vec![
            SystemPrincipalId::new("roll.registry").ok_or("issuer")?,
        ])),
    ]);
    let _ = disabled.take_module_ops();
    let error = disabled
        .try_modules_loaded(f.boot)
        .err()
        .ok_or("disabled update history must refuse")?;
    assert!(
        error
            .to_string()
            .contains("compatible-update history requires paired recovery")
    );
    assert!(disabled.take_module_ops().is_empty());
    Ok(())
}

#[test]
fn checkpoint_requires_exact_both_direction_fee_key_join_and_replay_poison_is_sticky() -> Result {
    let mut f = Fixture::new()?;
    let p = f.update(2)?;
    f.publish(f.rules.update.clone(), 2, p)?;
    let audit = f.state().update_receipt_audit();
    let subject = audit.receipts()?.next().ok_or("receipt")?;
    let row = crate::checkpoint::IdemRow {
        ns: receipt::NAMESPACE.into(),
        key: subject.commit.to_hex().into(),
        plan_hash: subject
            .plan_hash
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
        tx: subject.transaction,
        posted_at: subject.posted_at_utc,
        seq: subject.journal_seq,
    };
    assert!(audit.checkpoint_complete(std::slice::from_ref(&row)));
    assert!(!audit.checkpoint_complete(&[]));
    assert!(!audit.checkpoint_complete(&[row.clone(), row.clone()]));
    let mut crossed = row.clone();
    crossed.tx.0 += 1;
    assert!(!audit.checkpoint_complete(&[crossed]));
    assert!(!UpdateReceiptAudit::default().checkpoint_complete(&[row]));
    assert!(UpdateReceiptAudit::default().checkpoint_complete(&[]));
    let tx = f
        .state()
        .transaction(subject.transaction)
        .ok_or("transaction")?;
    for change in 0..4 {
        let mut bad = tx.clone();
        match change {
            0 => bad.idempotency.namespace = "other".into(),
            1 => {
                let EconomicCause::Ruleset { namespace, .. } = &mut bad.cause else {
                    return Err("cause".into());
                };
                *namespace = "other".into();
            }
            2 => bad.actor = PrincipalId::Object(f.actor),
            _ => bad.lines.get_mut(0).ok_or("line")?.delta = Amount::ZERO,
        };
        let mut replay = UpdateReceiptAudit::default();
        replay.observe(&bad);
        assert!(replay.finish_replay().is_err());
        replay.observe(tx);
        assert!(replay.finish_replay().is_err());
    }
    Ok(())
}

#[test]
fn zero_fee_update_registration_refuses_before_any_owner_state_exists() -> Result {
    let rules = rules()?;
    assert!(
        mush_core::Engine::with_modules(
            World::new_minimal(),
            mush_core::Registry::new(),
            modules(&rules, 0)
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn approval_and_ordinary_post_routes_cannot_bypass_the_closed_fee_owner() -> Result {
    let mut f = Fixture::new()?;
    f.state_mut()
        .snapshot
        .account_mut(AccountId(1))
        .ok_or("payer")?
        .approval = Some(crate::record::ApprovalRule {
        threshold: Amount::from_minor(99),
        policy: mush_org::ApprovalPolicy::NamedSet {
            approvals: 1,
            actors: std::collections::BTreeSet::from([GOD]),
        },
        window: None,
    });
    let before = f.state().snapshot.clone();
    let next = f.state().next_transaction;
    assert!(f.update(2).is_err());
    assert_eq!(f.state().snapshot, before);
    assert_eq!(f.state().next_transaction, next);
    f.state_mut()
        .snapshot
        .account_mut(AccountId(1))
        .ok_or("payer")?
        .approval = None;
    struct Permit;
    impl AuthEval for Permit {
        fn authorize(&self, _: &PrincipalId, _: &Account, _: Amount) -> bool {
            true
        }
    }
    for posted in [false, true] {
        if posted {
            let p = f.update(2)?;
            f.publish(f.rules.update.clone(), 2, p)?;
        }
        let request = RegistryCompatibleUpdate {
            charter: f.shell,
            title_revision: 1,
            payer: AccountId(1),
        };
        let plan = plan(&policy(100), f.actor, token(2), &request);
        let gate = f.state().key_gate();
        let env = ValidateEnv {
            auth: &Permit,
            custody: &NoCustody,
            facts: &NoFacts,
            now: f.world.world().now(),
            ic_now: f.world.world().game_now_unix(),
            next_hold_id: f.state().next_hold,
            spent: BTreeMap::new(),
            keys: &gate,
        };
        assert!(matches!(
            crate::engine::validate(&plan, &f.state().snapshot, &env),
            PlanResult::Refused(crate::EconomyError::ManagedContract)
        ));
        if posted {
            let tx = f
                .state()
                .update_receipt_audit()
                .receipts()?
                .next()
                .ok_or("receipt")?
                .transaction;
            assert_eq!(
                crate::contract::managed::require_legacy_transaction(
                    f.state().transaction(tx).ok_or("transaction")?,
                    &f.state().snapshot
                ),
                Err(crate::EconomyError::ManagedContract)
            );
        }
    }
    Ok(())
}

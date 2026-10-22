#![cfg(test)]

//! Native payment/title owner contract, driven through actual Engine composition.
//! This test provider is not the production signed Roll resale offer protocol.
use super::*;
use mush_core::action::{
    ActionError, ActionErrorCode, ActionInvocation, ActionInvocationDraft, ActionOfferId,
    ActionProvider, AuthoritativeActionOutput, CanonicalHash, IdempotencyKey as ActionKey,
    IdempotencyScope, PerceivedRef, PreparedAuthoritativeAction, PreparedCausalRef,
    PrincipalId as ActionPrincipal, RequestUid, ScopedIdempotencyKey,
};
use mush_core::action_state::{StateFamilyDescriptor, StateFamilyHandle, StateFamilyId};
use mush_core::authoritative::{AuthoritativeBundle, AuthoritativeBundleDraft};
use mush_core::participant::ParticipantHandle;
use mush_core::{
    ActionTxn, CanonicalId, DefinitionDescriptor, DefinitionId, DefinitionKind,
    DefinitionRegistryBuilder, DefinitionValidationContext, GOD, GameModule, ModError,
    ModuleBootData, ModuleOp, ModuleWrite, ObjKind, PrepareCtx, Registrar, RevisionSet, RulesValue,
};
use mush_testkit::{TestWorld, fold_module_ops_into};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
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
        json!({"fixture":"native-charter-sale"}),
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
    let wrong = add("action", "wrong_transfer")?;
    let outcome = add("outcome", "charter_sold")?;
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
    seller: ObjId,
    seller_account: AccountId,
    offer: CommitToken,
    offer_digest: DefinitionDigest,
    price: Amount,
}
// Strict test-only typed evidence derived from each actual prepared Economy
// output. It is not a production Roll filing/update record or an authority.
const RECEIPT_FAMILY: &str = "charter_sale_fixture_receipt";
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "subject",
    rename_all = "snake_case",
    deny_unknown_fields
)]
enum FixtureEffect {
    Initial(Box<super::super::receipt::InitialFilingAuditSubject>),
    Transfer(Box<CharterTransferAuditSubject>),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureReceipt {
    format: u8,
    action: DefinitionRef,
    commit: CommitToken,
    actor: ObjId,
    effect: FixtureEffect,
}
impl FixtureReceipt {
    fn validate(
        &self,
        initial: &DefinitionRef,
        update: &DefinitionRef,
    ) -> std::result::Result<(), String> {
        if self.format != 1 || self.actor.0 == 0 {
            return Err("fixture receipt shape".into());
        }
        match &self.effect {
            FixtureEffect::Initial(s) if &self.action == initial && s.commit() == self.commit => {
                let data = serde_json::to_value(s).map_err(|_| "initial encoding")?;
                if data.get("issued_owner") != Some(&json!(PrincipalId::Object(self.actor))) {
                    return Err("initial actor differs".into());
                }
            }
            FixtureEffect::Transfer(s)
                if &self.action == update && s.commit == self.commit && s.buyer == self.actor =>
            {
                s.validate().map_err(|_| "update shape")?;
            }
            _ => return Err("fixture receipt affinity".into()),
        }
        Ok(())
    }
    fn verify(&self, state: &EconomyStore) -> Result {
        match &self.effect {
            FixtureEffect::Initial(subject) => state.filing_receipt_audit().verify(subject)?,
            FixtureEffect::Transfer(subject) => {
                state.charter_transfer_receipt_audit().verify(subject)?
            }
        }
        Ok(())
    }
}
fn receipts(
    reg: &mut Registrar<'_>,
    rules: &Rules,
) -> std::result::Result<StateFamilyHandle<CommitToken, FixtureReceipt>, ModError> {
    let bad = |e: &dyn std::fmt::Display| ModError::msg(e.to_string());
    let initial = rules.initial.clone();
    let update = rules.update.clone();
    reg.state_family(
        StateFamilyDescriptor::new(
            StateFamilyId::new(RECEIPT_FAMILY).map_err(|e| bad(&e))?,
            CanonicalId::new("charter_sale_fixture_receipt.key.v1").map_err(|e| bad(&e))?,
            CanonicalId::new("charter_sale_fixture_receipt.value.v1").map_err(|e| bad(&e))?,
            1,
            CanonicalId::new("charter_sale_fixture_receipt.decode.v1").map_err(|e| bad(&e))?,
            CanonicalId::new("charter_sale_fixture_receipt.validate.v1").map_err(|e| bad(&e))?,
            CanonicalId::new("charter_sale_fixture_receipt.canonical.v1").map_err(|e| bad(&e))?,
        )
        .map_err(|e| bad(&e))?
        .with_limits(32, 16_384)
        .map_err(|e| bad(&e))?,
        |key: &CommitToken| Ok(key.to_hex()),
        |value: &Value| {
            let row: FixtureReceipt = serde_json::from_value(value.clone())
                .map_err(|_| "fixture receipt encoding".to_owned())?;
            if serde_json::to_value(&row).map_err(|_| "fixture receipt encoding".to_owned())?
                != *value
            {
                return Err("fixture receipt exact shape".into());
            }
            Ok(row)
        },
        move |row: &FixtureReceipt| row.validate(&initial, &update),
        |row: &FixtureReceipt| {
            serde_json::to_value(row).map_err(|_| "fixture receipt encoding".into())
        },
    )
}
fn prepared_receipt(
    writes: &[ReplayWrite],
    rules: &Rules,
    action: &DefinitionRef,
    commit: CommitToken,
    actor: ObjId,
) -> Result<FixtureReceipt> {
    let mut found = None;
    for write in writes {
        let ReplayWrite::ModuleJsonSet {
            namespace,
            key,
            value,
        } = write
        else {
            continue;
        };
        if namespace.as_str() != "roll" {
            continue;
        }
        let receipt: FixtureReceipt =
            serde_json::from_value(value.get("value").ok_or("typed fixture value")?.clone())?;
        receipt.validate(&rules.initial, &rules.update)?;
        let expected = json!({"family":{"owner":"roll","id":RECEIPT_FAMILY},"key":receipt.commit.to_hex(),"schema":{"id":"charter_sale_fixture_receipt.value.v1","version":1},"value":receipt});
        if value != &expected
            || key.as_str() != format!("action-state/v1/{RECEIPT_FAMILY}/{}", commit.to_hex())
            || &receipt.action != action
            || receipt.commit != commit
            || receipt.actor != actor
            || found.is_some()
        {
            return Err("unexpected/crossed typed fixture write".into());
        }
        found = Some(receipt);
    }
    found.ok_or_else(|| "actual composed fixture receipt missing".into())
}
struct Provider {
    receipts: StateFamilyHandle<CommitToken, FixtureReceipt>,
    action: DefinitionRef,
    handle: ParticipantHandle<InitialFilingParticipant>,
    initial: bool,
    outcome: DefinitionRef,
}
fn error() -> ActionError {
    ActionError::new(
        ActionErrorCode::ProviderRejected,
        false,
        "charter sale fixture refused",
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
        txn: &mut ActionTxn<'_>,
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
            EconomyRequest::CharterTransfer(RegistryCharterTransfer {
                charter: r.charter,
                payer: r.payer,
                title_revision: r.title_revision,
                seller: r.seller,
                buyer: inv.actor(),
                seller_account: r.seller_account,
                offer: r.offer,
                offer_digest: r.offer_digest,
                price: r.price,
            })
        };
        let output = ctx
            .prepare_owner(&self.handle, request)
            .map_err(|_| error())?;
        let effect = if self.initial {
            let output = output.initial_filing().map_err(|_| error())?;
            let title = output.title();
            FixtureEffect::Initial(Box::new(
                super::super::receipt::InitialFilingAuditSubject::new(
                    output.commit_token(),
                    output.transaction(),
                    output.journal_seq(),
                    title.object,
                    title.title_owner.clone(),
                    title.revision,
                    output.payer(),
                    output.sink(),
                    output.fee(),
                    output.issuer().clone(),
                    title.catalog.clone().ok_or_else(error)?,
                )
                .map_err(|_| error())?,
            ))
        } else {
            FixtureEffect::Transfer(Box::new(
                output
                    .charter_transfer()
                    .map_err(|_| error())?
                    .subject()
                    .clone(),
            ))
        };
        let value = serde_json::to_value(&effect).map_err(|_| error())?;
        txn.insert(
            &self.receipts,
            &ctx.commit_token(),
            FixtureReceipt {
                format: 1,
                action: self.action.clone(),
                commit: ctx.commit_token(),
                actor: inv.actor(),
                effect,
            },
        )
        .map_err(|_| error())?;
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
        let receipts = receipts(reg, self)?;
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
                action.clone(),
                Provider {
                    receipts: receipts.clone(),
                    action,
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
                .with_mind_charter_transfers(r.update.clone()),
        ),
    ]
}
struct Fixture {
    world: TestWorld,
    rules: Rules,
    actor: ObjId,
    buyer: ObjId,
    shell: ObjId,
    boot: Boot,
    staged: Option<FixtureReceipt>,
}
impl Fixture {
    fn empty() -> Result<Self> {
        let rules = rules()?;
        let mut world = TestWorld::with_modules(modules(&rules, 100));
        let actor = world.spawn_player("Seller");
        let buyer = world.spawn_player("Buyer");
        let shell =
            world
                .world_mut()
                .create(ObjKind::Thing, "Reserved native charter", GOD, None)?;
        Ok(Self {
            world,
            rules,
            actor,
            buyer,
            shell,
            boot: Boot::new(),
            staged: None,
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
            format!("@account/open #{}=cr", f.buyer.0),
            "@econ/mint 0=3 10000".to_owned(),
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
        let commit = token(n);
        let request = RequestUid::from_bytes([n; 16]);
        let principal = ActionPrincipal::Object { id: actor };
        let inv = ActionInvocation::new(ActionInvocationDraft {
            request,
            principal: principal.clone(),
            actor,
            offered_action: ActionOfferId(CanonicalHash::from_bytes([2; 32])),
            subject: PerceivedRef::SelfActor,
            targets: vec![],
            parameters: json!({"charter":self.shell,"payer":if action==&self.rules.initial {AccountId(1)} else {AccountId(3)},"title_revision":self.state().snapshot.asset(self.shell).map_or(0,|a|a.revision),
                "seller":self.actor,"seller_account":AccountId(1),"offer":token(200),"offer_digest":DefinitionDigest::from_bytes([44;32]),"price":Amount::from_minor(500)}),
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal,
                    owner: OwnerId::new("roll")?,
                    namespace: CanonicalId::new("charter_transfer_fixture")?,
                },
                1,
                ActionKey::Request(request),
            ),
        })?;
        let (p, o) = self
            .world
            .engine_mut()
            .prepare_registered_composed_authoritative_action(action, &inv, commit)?;
        Ok((p, o, inv))
    }
    fn update(&mut self, n: u8) -> Result<PreparedAction> {
        let action = self.rules.update.clone();
        self.prepare(&action, self.buyer, n)
    }
    fn stage(
        &mut self,
        action: DefinitionRef,
        n: u8,
        p: PreparedAction,
    ) -> Result<Vec<ReplayWrite>> {
        let (p, o, inv) = p;
        let writes = p.authoritative_replay_writes();
        let receipt = prepared_receipt(&writes, &self.rules, &action, token(n), inv.actor())?;
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
        self.staged = Some(receipt);
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
        self.staged
            .take()
            .ok_or("prepared typed receipt")?
            .verify(self.state())?;
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
fn native_sale_commits_buyer_money_and_title_together_preserving_birth() -> Result {
    let mut f = Fixture::new()?;
    let before = f.state().snapshot.clone();
    let before_birth = serde_json::to_value(f.state().filing_receipt_audit())?;
    let title = before.asset(f.shell).ok_or("title")?.clone();
    let p = f.update(2)?;
    check_eq!(f.state().snapshot, before);
    let writes = f.stage(f.rules.update.clone(), 2, p)?;
    check_eq!(f.state().snapshot, before);
    f.take_install()?;
    let subject = f
        .state()
        .charter_transfer_receipt_audit()
        .receipts()?
        .next()
        .ok_or("actual native receipt")?
        .clone();
    f.state()
        .charter_transfer_receipt_audit()
        .verify(&subject)?;
    check_eq!(subject.seller, f.actor);
    check_eq!(subject.buyer, f.buyer);
    check_eq!(subject.title_revision, title.revision);
    check_eq!(subject.title.revision, title.revision + 1);
    check_eq!(subject.title.catalog, title.catalog);
    check_eq!(subject.title.title_owner, PrincipalId::Object(f.buyer));
    check_eq!(f.state().snapshot.asset(f.shell), Some(&subject.title));
    check_eq!(
        serde_json::to_value(f.state().filing_receipt_audit())?,
        before_birth
    );
    check_eq!(f.state().snapshot.managed_titles, before.managed_titles);
    for (id, value) in [(1, 10_400), (2, 100), (3, 9500)] {
        check_eq!(
            f.state()
                .snapshot
                .account(AccountId(id))
                .ok_or("account")?
                .posted_balance,
            Amount::from_minor(value)
        );
    }
    check_eq!(
        writes.len(),
        7,
        "native journal, two accounts, title, key, meta and typed witness"
    );
    let tx = f
        .state()
        .transaction(subject.transaction)
        .ok_or("native transaction")?;
    check_eq!(receipt::subject_of(tx)?, subject);
    let mut replay = CharterTransferReceiptAudit::default();
    let mut history: Vec<_> = f.state().transactions.values().collect();
    history.sort_by_key(|t| t.journal_seq);
    for entry in history {
        replay.observe(entry);
    }
    check!(replay.verify(&subject).is_err());
    replay.finish_replay()?;
    replay.verify(&subject)?;
    check!(
        f.update(3).is_err(),
        "old seller never retains transfer authority"
    );
    Ok(())
}

#[test]
fn sale_rechecks_title_money_and_birth_at_both_custody_boundaries() -> Result {
    for at_take in [false, true] {
        for change in 0..4 {
            let mut f = Fixture::new()?;
            let p = f.update(2)?;
            let action = f.rules.update.clone();
            let mut prepared = Some(p);
            if at_take {
                f.stage(action.clone(), 2, prepared.take().ok_or("preparation")?)?;
            }
            let shell = f.shell;
            match change {
                0 => {
                    f.state_mut()
                        .snapshot
                        .asset_mut(shell)
                        .ok_or("title")?
                        .revision += 1
                }
                1 => {
                    f.state_mut()
                        .snapshot
                        .account_mut(AccountId(3))
                        .ok_or("payer")?
                        .status = AccountStatus::Frozen
                }
                2 => {
                    f.state_mut().snapshot.managed_titles =
                        crate::managed_title::ManagedTitleIndex::Unavailable
                }
                _ => f
                    .world
                    .world_mut()
                    .get_mut(f.actor)
                    .ok_or("seller")?
                    .flags
                    .insert(mush_core::Flags::GOING),
            }
            let before = f.state().snapshot.clone();
            let next = f.state().next_transaction;
            if at_take {
                check!(f.world.engine_mut().take_staged_action().is_err());
            } else {
                check!(
                    f.stage(action, 2, prepared.take().ok_or("preparation")?)
                        .is_err()
                );
            }
            check_eq!(f.state().snapshot, before);
            check_eq!(f.state().next_transaction, next);
            check_eq!(
                f.state().charter_transfer_receipt_audit().receipts()?.len(),
                0
            );
        }
    }
    Ok(())
}

#[test]
fn sale_requires_actual_buyer_personal_payer_and_exact_registered_action() -> Result {
    let mut f = Fixture::new()?;
    let action = f.rules.update.clone();
    let seller = f.actor;
    let buyer = f.buyer;
    let other = f.world.spawn_player("Unrelated buyer");
    check!(f.prepare(&action, seller, 2).is_err());
    check!(f.prepare(&action, other, 2).is_err());
    let wrong = f.rules.wrong.clone();
    check!(f.prepare(&wrong, buyer, 2).is_err());
    for id in [1, 3] {
        let original = f
            .state()
            .snapshot
            .account(AccountId(id))
            .ok_or("account")?
            .clone();
        f.state_mut()
            .snapshot
            .account_mut(AccountId(id))
            .ok_or("account")?
            .owner = PrincipalId::Object(other);
        check!(f.update(2).is_err());
        *f.state_mut()
            .snapshot
            .account_mut(AccountId(id))
            .ok_or("account")? = original;
    }
    f.state_mut()
        .snapshot
        .account_mut(AccountId(3))
        .ok_or("payer")?
        .approval = Some(crate::record::ApprovalRule {
        threshold: Amount::from_minor(499),
        policy: mush_org::ApprovalPolicy::NamedSet {
            approvals: 1,
            actors: std::collections::BTreeSet::from([GOD]),
        },
        window: None,
    });
    let before = f.state().snapshot.clone();
    check!(f.update(2).is_err());
    check_eq!(f.state().snapshot, before);
    check_eq!(
        f.state().charter_transfer_receipt_audit().receipts()?.len(),
        0
    );
    Ok(())
}

#[test]
fn sale_cold_replay_preserves_exact_title_and_requires_paired_recovery() -> Result {
    let mut f = Fixture::new()?;
    let p = f.update(2)?;
    f.publish(f.rules.update.clone(), 2, p)?;
    let subject = f
        .state()
        .charter_transfer_receipt_audit()
        .receipts()?
        .next()
        .ok_or("receipt")?
        .clone();
    let mut cold = Fixture::empty()?;
    check_eq!(cold.shell, f.shell);
    check_eq!(cold.buyer, f.buyer);
    cold.world.try_modules_loaded(f.boot.clone())?;
    cold.state()
        .charter_transfer_receipt_audit()
        .verify(&subject)?;
    check_eq!(
        cold.state().snapshot.asset(f.shell),
        f.state().snapshot.asset(f.shell)
    );
    check_eq!(
        cold.state().snapshot.managed_titles,
        f.state().snapshot.managed_titles
    );
    check_eq!(cold.state().filing_receipt_audit().receipts()?.len(), 1);
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
        .ok_or("disabled sale history must refuse")?;
    check!(
        error
            .to_string()
            .contains("charter-transfer history requires paired recovery")
    );
    check!(disabled.take_module_ops().is_empty());
    Ok(())
}

#[test]
fn sale_history_checks_complete_inverse_and_rejects_crossed_native_title() -> Result {
    let mut f = Fixture::new()?;
    let p = f.update(2)?;
    f.publish(f.rules.update.clone(), 2, p)?;
    let audit = f.state().charter_transfer_receipt_audit();
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
    let mut rows = journal_rows(&f);
    check!(audit.checkpoint_complete(&rows));
    check!(
        !audit.checkpoint_complete(std::slice::from_ref(&row)),
        "filing inverse missing"
    );
    check!(!audit.checkpoint_complete(&[]));
    check!(!audit.checkpoint_complete(&[row.clone(), row.clone()]));
    check!(!CharterTransferReceiptAudit::default().checkpoint_complete(&[row]));
    check!(CharterTransferReceiptAudit::default().checkpoint_complete(&[]));
    let tx = f
        .state()
        .transaction(subject.transaction)
        .ok_or("transaction")?;
    rows.retain(|r| r.ns.as_str() != receipt::NAMESPACE);
    check!(!audit.checkpoint_complete(&rows));
    for change in 0..7 {
        let mut bad = tx.clone();
        match change {
            0 => bad.idempotency.namespace = "foreign".into(),
            1 => bad.actor = PrincipalId::Object(f.actor),
            2 => bad.lines.get_mut(0).ok_or("line")?.delta = Amount::ZERO,
            3 => {
                bad.asset_effects
                    .get_mut(0)
                    .ok_or("effect")?
                    .post
                    .title_owner = PrincipalId::Object(f.actor)
            }
            4 => bad.asset_effects.get_mut(0).ok_or("effect")?.post.revision += 1,
            5 => {
                bad.asset_effects
                    .get_mut(0)
                    .ok_or("effect")?
                    .post
                    .acquisition = None
            }
            _ => bad.asset_effects.get_mut(0).ok_or("effect")?.post.catalog = None,
        }
        let mut replay = CharterTransferReceiptAudit::default();
        let mut history: Vec<_> = f
            .state()
            .transactions
            .values()
            .filter(|t| t.journal_seq < tx.journal_seq)
            .collect();
        history.sort_by_key(|t| t.journal_seq);
        for entry in history {
            replay.observe(entry);
        }
        replay.observe(&bad);
        check!(replay.finish_replay().is_err());
        replay.observe(tx);
        check!(replay.finish_replay().is_err());
    }
    Ok(())
}

fn journal_rows(f: &Fixture) -> Vec<crate::checkpoint::IdemRow> {
    f.state()
        .transactions
        .values()
        .map(|tx| crate::checkpoint::IdemRow {
            ns: tx.idempotency.namespace.clone(),
            key: tx.idempotency.key.clone(),
            plan_hash: tx.plan_hash.iter().map(|b| format!("{b:02x}")).collect(),
            tx: tx.id,
            posted_at: tx.posted_at,
            seq: tx.journal_seq,
        })
        .collect()
}

#[test]
fn legacy_managed_title_mutation_cannot_disappear_from_native_namespace_census() -> Result {
    let mut f = Fixture::new()?;
    let before_sale = f.state().charter_transfer_receipt_audit().clone();
    let p = f.update(2)?;
    f.publish(f.rules.update.clone(), 2, p)?;
    let subject = f
        .state()
        .charter_transfer_receipt_audit()
        .receipts()?
        .next()
        .ok_or("sale")?;
    let mut old_generic = f
        .state()
        .transaction(subject.transaction)
        .ok_or("transaction")?
        .clone();
    old_generic.idempotency = IdempotencyKey::new("ordinary", "old-sale");
    old_generic.cause = EconomicCause::Ruleset {
        namespace: "ordinary".into(),
        correlation: "old-sale".into(),
    };
    let mut audit = before_sale.clone();
    audit.observe(&old_generic);
    check!(
        audit.finish_replay().is_err(),
        "old generic transfer must poison managed lineage"
    );
    for kind in [
        crate::AssetEffectKind::CatalogSet,
        crate::AssetEffectKind::WrittenOff,
        crate::AssetEffectKind::Abandoned,
        crate::AssetEffectKind::Encumbered,
    ] {
        let mut mutation = old_generic.clone();
        mutation.asset_effects.get_mut(0).ok_or("effect")?.kind = kind;
        let mut audit = before_sale.clone();
        audit.observe(&mutation);
        check!(audit.finish_replay().is_err());
    }
    let rows = journal_rows(&f);
    let mut missing = serde_json::to_value(f.state().charter_transfer_receipt_audit())?;
    missing.as_object_mut().ok_or("audit")?.remove("lineages");
    let missing: CharterTransferReceiptAudit = serde_json::from_value(missing)?;
    check!(!missing.checkpoint_complete(&rows));
    check!(!CharterTransferReceiptAudit::default().checkpoint_complete(&rows));
    Ok(())
}

#[test]
fn public_post_and_exact_key_retries_never_gain_private_title_authority() -> Result {
    let mut f = Fixture::new()?;
    for posted in [false, true] {
        if posted {
            let p = f.update(2)?;
            f.publish(f.rules.update.clone(), 2, p)?;
        }
        let request = RegistryCharterTransfer {
            offer: token(200),
            offer_digest: DefinitionDigest::from_bytes([44; 32]),
            charter: f.shell,
            title_revision: 1,
            seller: f.actor,
            buyer: f.buyer,
            payer: AccountId(3),
            seller_account: AccountId(1),
            price: Amount::from_minor(500),
        };
        let plan = plan(&policy(100), f.buyer, token(2), &request);
        let gate = f.state().key_gate();
        let env = ValidateEnv {
            auth: &crate::AllowAll,
            custody: &NoCustody,
            facts: &NoFacts,
            now: f.world.world().now(),
            ic_now: f.world.world().game_now_unix(),
            next_hold_id: f.state().next_hold,
            spent: BTreeMap::new(),
            keys: &gate,
        };
        check!(matches!(
            crate::engine::validate(&plan, &f.state().snapshot, &env),
            PlanResult::Refused(crate::EconomyError::ManagedContract)
        ));
    }
    Ok(())
}

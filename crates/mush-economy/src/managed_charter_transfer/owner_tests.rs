//! Actual registered Economy owner funding/terminal/recovery tests. The typed
//! Roll fixture is inert evidence of the actual opaque owner output. It does not
//! claim production seller offers, claim census, Registry sanction or service intent.
use super::*;
use crate::record::{AccountStatus, CatalogRef, EconomicCause, HoldStatus};
use crate::registry_filing::InitialFilingAuditSubject;
use crate::registry_filing::{
    EconomyRequest, InitialFilingParticipant, InitialFilingPolicy, RegistryInitialFiling,
};
use crate::{AccountId, Amount, EconomyStore, InstrumentId};
use mush_core::action::{
    ActionError, ActionErrorCode, ActionInvocation, ActionInvocationDraft, ActionOfferId,
    ActionProvider, AuthoritativeActionOutput, CanonicalHash, IdempotencyKey as ActionKey,
    IdempotencyScope, PerceivedRef, PreparedAuthoritativeAction, PreparedCausalRef,
    PrincipalId as ActionPrincipal, RequestUid, ScopedIdempotencyKey,
};
use mush_core::action_state::{StateFamilyDescriptor, StateFamilyHandle, StateFamilyId};
use mush_core::authoritative::ReplayWrite;
use mush_core::authoritative::{AuthoritativeBundle, AuthoritativeBundleDraft};
use mush_core::mechanics::OwnerId;
use mush_core::participant::ParticipantHandle;
use mush_core::{
    ActionTxn, CanonicalId, DefinitionDescriptor, DefinitionId, DefinitionKind,
    DefinitionRegistryBuilder, DefinitionValidationContext, GOD, GameModule, ModError,
    ModuleBootData, ModuleOp, ModuleWrite, ObjKind, PrepareCtx, Registrar, RevisionSet, RulesValue,
};
use mush_core::{CommitToken, DefinitionDigest, DefinitionRef, ObjId};
use mush_org::{PrincipalId, SystemPrincipalId};
use mush_testkit::{TestWorld, fold_module_ops_into};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use smol_str::SmolStr;
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
    offer: DefinitionRef,
    immediate: DefinitionRef,
    fund: DefinitionRef,
    settle: DefinitionRef,
    refund: DefinitionRef,
    revoke: DefinitionRef,
    wrong: DefinitionRef,
    outcome: DefinitionRef,
}
fn descriptor(kind: &str, id: &str, version: u32) -> Result<DefinitionDescriptor> {
    Ok(DefinitionDescriptor::new(
        DefinitionKind::new(kind)?,
        DefinitionId::new(id)?,
        version,
        json!({"fixture":"native-charter-pending"}),
    )?)
}
fn rules() -> Result<Rules> {
    let mut b = DefinitionRegistryBuilder::new();
    let owner = OwnerId::new("roll")?;
    let mut declarations = Vec::new();
    let mut add = |kind, id, version| -> Result<DefinitionRef> {
        let d = descriptor(kind, id, version)?;
        let r = b.register(owner.clone(), d.clone())?;
        declarations.push(d);
        Ok(r)
    };
    let initial = add("action", "initial_filing", 1)?;
    let offer = add("action", OFFER_ACTION, 2)?;
    let immediate = add("action", "accept_mind_charter_transfer", 1)?;
    let fund = add("action", FUND_ACTION, 1)?;
    let settle = add("action", SETTLE_ACTION, 1)?;
    let refund = add("action", REFUND_ACTION, 1)?;
    let revoke = add("action", "registry_term_revocation", 1)?;
    let wrong = add("action", "wrong_pending_transfer", 1)?;
    let outcome = add("outcome", "charter_pending_fixture", 1)?;
    Ok(Rules {
        declarations,
        initial,
        offer,
        immediate,
        fund,
        settle,
        refund,
        revoke,
        wrong,
        outcome,
    })
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Input {
    Initial {
        shell: ObjId,
        payer: AccountId,
    },
    Fund {
        terms: CharterPendingTermsV1,
        payer: AccountId,
    },
    Terminal {
        funding: CommitToken,
        reason: CharterPendingDecisionKindV1,
    },
}
// Strict test-only typed evidence derived from each actual prepared Economy
// output. It is not a production Roll filing/update record or an authority.
const RECEIPT_FAMILY: &str = "charter_pending_fixture_receipt";
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "subject",
    rename_all = "snake_case",
    deny_unknown_fields
)]
enum FixtureEffect {
    Initial(InitialFilingAuditSubject),
    Pending(CharterPendingHistory),
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
    fn validate(&self, initial: &DefinitionRef) -> std::result::Result<(), String> {
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
            FixtureEffect::Pending(h) => {
                h.validate().map_err(|_| "pending history shape")?;
                let (action, commit, actor) = match h.terminal() {
                    None => (
                        &h.consent().effect,
                        h.consent().operation,
                        h.consent().terms.buyer,
                    ),
                    Some(t) => (
                        &t.decision().action,
                        t.decision().operation,
                        t.decision().actor,
                    ),
                };
                if &self.action != action || self.commit != commit || self.actor != actor {
                    return Err("pending output affinity".into());
                }
            }
            _ => return Err("fixture receipt affinity".into()),
        }
        Ok(())
    }
    fn verify(&self, state: &EconomyStore) -> Result {
        match &self.effect {
            FixtureEffect::Initial(subject) => state.filing_receipt_audit().verify(subject)?,
            FixtureEffect::Pending(h) => {
                if state.pending_charter_history(h.consent().operation)? != h {
                    return Err("actual native history differs from prepared output".into());
                }
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
    reg.state_family(
        StateFamilyDescriptor::new(
            StateFamilyId::new(RECEIPT_FAMILY).map_err(|e| bad(&e))?,
            CanonicalId::new("charter_pending_fixture_receipt.key.v1").map_err(|e| bad(&e))?,
            CanonicalId::new("charter_pending_fixture_receipt.value.v1").map_err(|e| bad(&e))?,
            1,
            CanonicalId::new("charter_pending_fixture_receipt.decode.v1").map_err(|e| bad(&e))?,
            CanonicalId::new("charter_pending_fixture_receipt.validate.v1").map_err(|e| bad(&e))?,
            CanonicalId::new("charter_pending_fixture_receipt.canonical.v1")
                .map_err(|e| bad(&e))?,
        )
        .map_err(|e| bad(&e))?
        .with_limits(128, 65_536)
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
        move |row: &FixtureReceipt| row.validate(&initial),
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
        receipt.validate(&rules.initial)?;
        let expected = json!({"family":{"owner":"roll","id":RECEIPT_FAMILY},"key":receipt.commit.to_hex(),"schema":{"id":"charter_pending_fixture_receipt.value.v1","version":1},"value":receipt});
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
    outcome: DefinitionRef,
}
fn error() -> ActionError {
    ActionError::new(
        ActionErrorCode::ProviderRejected,
        false,
        "charter pending owner fixture refused",
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
        let is_initial = matches!(&r, Input::Initial { .. });
        let request = match r {
            Input::Initial { shell, payer } => {
                EconomyRequest::InitialFiling(RegistryInitialFiling {
                    shell,
                    payer,
                    owner: PrincipalId::Object(inv.actor()),
                    scope: None,
                })
            }
            Input::Fund { terms, payer } => {
                EconomyRequest::CharterPendingFunding(CharterPendingFundingRequest { terms, payer })
            }
            Input::Terminal { funding, reason } => {
                let world = ctx.world().map_err(|_| error())?;
                EconomyRequest::CharterPendingTerminal(CharterPendingTerminalRequest {
                    funding,
                    decision: CharterPendingDecisionV1 {
                        format: 1,
                        operation: ctx.commit_token(),
                        action: self.action.clone(),
                        actor: inv.actor(),
                        at_ic: world.world_ic_now(),
                        posted_at_utc: world.now(),
                        reason,
                    },
                })
            }
        };
        let output = ctx
            .prepare_owner(&self.handle, request)
            .map_err(|_| error())?;
        let effect = if is_initial {
            let output = output.initial_filing().map_err(|_| error())?;
            let title = output.title();
            FixtureEffect::Initial(
                InitialFilingAuditSubject::new(
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
            )
        } else {
            FixtureEffect::Pending(
                output
                    .charter_pending()
                    .map_err(|_| error())?
                    .history()
                    .clone(),
            )
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
// The actual Economy pending configuration retains the immediate-transfer
// prerequisite grant. This owner-only fixture never exercises that action;
// register its exact provider while keeping it unavailable to invocations.
struct ImmediateUnavailable;
impl ActionProvider for ImmediateUnavailable {
    fn validate_invocation(&self, _: &ActionInvocation) -> std::result::Result<(), ActionError> {
        Err(error())
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
        reg.action_provider(self.immediate.clone(), ImmediateUnavailable)?;
        let receipts = receipts(reg, self)?;
        let fund_handle =
            reg.request_owner_participant::<InitialFilingParticipant>(self.fund.clone())?;
        for action in [
            &self.initial,
            &self.fund,
            &self.settle,
            &self.refund,
            &self.revoke,
        ] {
            let handle = if action == &self.fund {
                fund_handle.clone()
            } else {
                reg.request_owner_participant::<InitialFilingParticipant>(action.clone())?
            };
            reg.action_provider(
                action.clone(),
                Provider {
                    receipts: receipts.clone(),
                    action: action.clone(),
                    handle,
                    outcome: self.outcome.clone(),
                },
            )?;
        }
        let handle = fund_handle;
        reg.action_provider(
            self.wrong.clone(),
            Provider {
                receipts,
                action: self.wrong.clone(),
                handle,
                outcome: self.outcome.clone(),
            },
        )?;
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
                .with_mind_charter_transfers(r.immediate.clone())
                .with_mind_charter_pending(
                    CharterPendingConfig::new(
                        SystemPrincipalId::new("roll.registry").expect("issuer"),
                        CatalogRef {
                            ruleset: "roll".into(),
                            code: "mind_charter".into(),
                        },
                        GOD,
                        [r.fund.clone(), r.settle.clone(), r.refund.clone()],
                        Some(r.revoke.clone()),
                    )
                    .expect("exact pending config"),
                ),
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
        world.world_mut().set_now(100);
        world
            .world_mut()
            .set_game_clock(mush_core::GameClock::default());
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
        let p = f.prepare(
            &action,
            f.actor,
            1,
            Input::Initial {
                shell: f.shell,
                payer: AccountId(1),
            },
        )?;
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
    fn terms(&self) -> Result<CharterPendingTermsV1> {
        let w = self.world.world();
        let row = CharterPendingTermsV1 {
            format: 1,
            offer: token(200),
            offer_action: self.rules.offer.clone(),
            offer_digest: DefinitionDigest::from_bytes([44; 32]),
            charter: self.shell,
            birth: token(1),
            ownership: CharterOwnershipBasisV1 {
                generation: 1,
                operation: token(1),
                digest: DefinitionDigest::from_bytes([45; 32]),
            },
            owner_term: CharterTermBasisV1 {
                licence: 1,
                generation: 1,
                operation: token(1),
                digest: DefinitionDigest::from_bytes([46; 32]),
                term_end: w.world_ic_now().at + 1000,
                grace_end: w.world_ic_now().at + 2000,
            },
            title_revision: self
                .state()
                .snapshot
                .asset(self.shell)
                .ok_or("title")?
                .revision,
            seller: self.actor,
            buyer: self.buyer,
            seller_account: AccountId(1),
            instrument: InstrumentId::new("cr").ok_or("instrument")?,
            price: Amount::from_minor(500),
            offered_at_ic: w.world_ic_now(),
            offered_at_utc: w.now(),
            expires_at_utc: w.now() + 3600,
            settle_seconds: 60,
        };
        row.validate()?;
        Ok(row)
    }
    fn prepare(
        &mut self,
        action: &DefinitionRef,
        actor: ObjId,
        n: u8,
        input: Input,
    ) -> Result<PreparedAction> {
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
            parameters: serde_json::to_value(input)?,
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal,
                    owner: OwnerId::new("roll")?,
                    namespace: CanonicalId::new("charter_pending_fixture")?,
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
    fn fund(&mut self, n: u8) -> Result<PreparedAction> {
        self.prepare(
            &self.rules.fund.clone(),
            self.buyer,
            n,
            Input::Fund {
                terms: self.terms()?,
                payer: AccountId(3),
            },
        )
    }
    fn terminal(&mut self, n: u8, reason: CharterPendingDecisionKindV1) -> Result<PreparedAction> {
        let action = if reason.settles() {
            self.rules.settle.clone()
        } else {
            self.rules.refund.clone()
        };
        self.prepare(
            &action,
            GOD,
            n,
            Input::Terminal {
                funding: token(2),
                reason,
            },
        )
    }
    fn cold(&self, mut boot: Boot) -> Result<Self> {
        let mut f = Self::empty()?;
        assert_eq!(
            (f.shell, f.actor, f.buyer),
            (self.shell, self.actor, self.buyer)
        );
        f.world.world_mut().set_now(self.world.world().now());
        f.world
            .world_mut()
            .set_world_timeline(self.world.world().world_timeline().clone());
        let mut typed = Vec::new();
        for (namespace, data) in &mut boot {
            data.kv.retain(|(key, value)| {
                if mush_core::is_action_state_persistence_key(key) {
                    typed.push(mush_core::ActionStateBootRecord::new(
                        namespace.clone(),
                        key.clone(),
                        value.clone(),
                    ));
                    false
                } else {
                    true
                }
            });
        }
        f.world.engine_mut().hydrate_action_state(typed)?;
        f.world.try_modules_loaded(boot)?;
        let _ = f.world.take_module_ops();
        let _ = f.world.world_mut().take_dirty();
        Ok(f)
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
                _ => return Err("unexpected pending replay write".into()),
            });
        }
        fold_module_ops_into(&mut self.boot, ops);
        Ok(writes)
    }
}
fn token(n: u8) -> CommitToken {
    CommitToken::from_bytes([n; 16])
}

fn funded() -> Result<Fixture> {
    let mut f = Fixture::new()?;
    let p = f.fund(2)?;
    f.publish(f.rules.fund.clone(), 2, p)?;
    Ok(f)
}
fn posted(f: &Fixture, id: u64) -> Result<i64> {
    Ok(f.state()
        .snapshot
        .account(AccountId(id))
        .ok_or("account")?
        .posted_balance
        .minor())
}
fn history(f: &Fixture) -> Result<CharterPendingHistory> {
    Ok(f.state().pending_charter_history(token(2))?.clone())
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
fn pending_fund_installs_one_real_buyer_acceptance_and_hold_without_title_or_debit() -> Result {
    let mut f = Fixture::new()?;
    let before = f.state().snapshot.clone();
    let p = f.fund(2)?;
    assert!(f.state().snapshot == before, "preparation must not publish");
    let writes = f.stage(f.rules.fund.clone(), 2, p)?;
    assert!(f.state().snapshot == before, "staging must not publish");
    assert!(
        writes
            .iter()
            .any(|w| matches!(w, ReplayWrite::LedgerChunkSet { .. }))
    );
    f.take_install()?;
    let h = history(&f)?;
    let c = f
        .state()
        .snapshot
        .contract(h.contract_id())
        .ok_or("actual contract")?;
    let held = f
        .state()
        .snapshot
        .hold(h.funding().hold())
        .ok_or("actual hold")?;
    h.verify_documents(c, held)?;
    assert_eq!(c.consent.parties, vec![PrincipalId::Object(f.buyer)]);
    assert_eq!(c.acceptances.len(), 1);
    assert_eq!(
        c.acceptances.first().ok_or("acceptance")?.party,
        PrincipalId::Object(f.buyer)
    );
    assert!(c.consent.deadline.is_none());
    assert_eq!(held.status, HoldStatus::Held);
    assert!(held.expires.is_none());
    assert!(f.state().snapshot.asset(f.shell) == before.asset(f.shell));
    assert_eq!((posted(&f, 1)?, posted(&f, 3)?), (9900, 10000));
    assert_eq!(
        f.state()
            .snapshot
            .account(AccountId(3))
            .ok_or("payer")?
            .available(),
        Some(Amount::from_minor(9500))
    );
    assert!(h.terminal().is_none());
    // No service, claim or seller-consent authority is inferred from this
    // fixture's inert typed witness. Production Roll must prove those joins.
    Ok(())
}
#[test]
fn pending_deadline_is_positive_and_real_settlement_moves_title_and_money_once() -> Result {
    let mut f = funded()?;
    assert!(
        f.terminal(
            3,
            CharterPendingDecisionKindV1::DeadlineReached {
                next_transition: 1,
                open_owned: 2
            }
        )
        .is_err()
    );
    let h = history(&f)?;
    let deadline = h.consent().deadline.at;
    f.world.world_mut().set_now(deadline);
    let p = f.terminal(
        3,
        CharterPendingDecisionKindV1::DeadlineReached {
            next_transition: 1,
            open_owned: 2,
        },
    )?;
    f.publish(f.rules.settle.clone(), 3, p)?;
    let h = history(&f)?;
    let t = h.terminal().ok_or("actual terminal")?;
    assert!(t.decision().reason.settles());
    assert_eq!(
        f.state()
            .snapshot
            .asset(f.shell)
            .ok_or("title")?
            .title_owner,
        PrincipalId::Object(f.buyer)
    );
    assert_eq!((posted(&f, 1)?, posted(&f, 3)?), (10400, 9500));
    assert_eq!(
        f.state()
            .snapshot
            .account(AccountId(3))
            .ok_or("payer")?
            .held_amount,
        Amount::ZERO
    );
    assert!(matches!(
        f.state()
            .snapshot
            .hold(h.funding().hold())
            .ok_or("hold")?
            .status,
        HoldStatus::Consumed { .. }
    ));
    assert!(
        f.terminal(
            4,
            CharterPendingDecisionKindV1::LastClaimClosed { next_transition: 2 }
        )
        .is_err()
    );
    let cold = f.cold(f.boot.clone())?;
    assert!(history(&cold)? == h);
    assert!(cold.state().snapshot.asset(f.shell) == f.state().snapshot.asset(f.shell));
    assert_eq!(
        cold.state()
            .charter_transfer_receipt_audit()
            .receipts()?
            .len(),
        0,
        "immediate v1 receipt namespace stays distinct"
    );
    Ok(())
}
#[test]
fn pending_last_claim_close_and_already_due_term_cap_settle_without_waiting_more() -> Result {
    let mut f = funded()?;
    let p = f.terminal(
        3,
        CharterPendingDecisionKindV1::LastClaimClosed { next_transition: 8 },
    )?;
    f.publish(f.rules.settle.clone(), 3, p)?;
    assert_eq!(posted(&f, 3)?, 9500);
    let mut f = Fixture::new()?;
    let mut terms = f.terms()?;
    terms.owner_term.term_end = terms.offered_at_ic.at - 1;
    terms.owner_term.grace_end = terms.offered_at_ic.at + 100;
    let p = f.prepare(
        &f.rules.fund.clone(),
        f.buyer,
        2,
        Input::Fund {
            terms,
            payer: AccountId(3),
        },
    )?;
    f.publish(f.rules.fund.clone(), 2, p)?;
    assert!(
        history(&f)?
            .consent()
            .deadline_reached(&f.world.world().world_ic_now())?
    );
    let p = f.terminal(
        3,
        CharterPendingDecisionKindV1::DeadlineReached {
            next_transition: 3,
            open_owned: 1,
        },
    )?;
    f.publish(f.rules.settle.clone(), 3, p)?;
    assert_eq!(posted(&f, 3)?, 9500);
    Ok(())
}
#[test]
fn pending_clock_and_recipient_refunds_release_real_hold_without_transfer() -> Result {
    for changed_clock in [false, true] {
        let mut f = funded()?;
        let title = f.state().snapshot.asset(f.shell).ok_or("title")?.clone();
        let reason = if changed_clock {
            let epoch = f.world.world().next_world_timeline_epoch(2)?;
            f.world.world_mut().append_world_timeline_epoch(epoch)?;
            CharterPendingDecisionKindV1::ClockChanged
        } else {
            let buyer = f.buyer;
            f.world
                .world_mut()
                .get_mut(buyer)
                .ok_or("buyer")?
                .flags
                .insert(mush_core::Flags::GOING);
            // Model an already-published external lifecycle change, so the
            // refund reaches actual owner custody rather than DirtyWorld.
            let _ = f.world.world_mut().take_dirty();
            CharterPendingDecisionKindV1::RecipientUnavailable
        };
        assert!(
            f.terminal(
                3,
                CharterPendingDecisionKindV1::LastClaimClosed { next_transition: 2 }
            )
            .is_err()
        );
        let p = f.terminal(3, reason)?;
        f.publish(f.rules.refund.clone(), 3, p)?;
        assert_eq!((posted(&f, 1)?, posted(&f, 3)?), (9900, 10000));
        assert!(f.state().snapshot.asset(f.shell) == Some(&title));
        let h = history(&f)?;
        assert_eq!(
            f.state()
                .snapshot
                .hold(h.funding().hold())
                .ok_or("hold")?
                .status,
            HoldStatus::Released
        );
        let cold = f.cold(f.boot.clone())?;
        assert!(history(&cold)? == h);
    }
    Ok(())
}
#[test]
fn pending_exact_registry_term_refund_is_in_same_actual_owner_commit() -> Result {
    let mut f = funded()?;
    let basis = history(&f)?.consent().terms.owner_term.clone();
    let reason = CharterPendingDecisionKindV1::RegistryRevoked {
        sanction: token(3),
        licence: basis.licence,
        generation: basis.generation,
        term_digest: basis.digest,
    };
    let p = f.prepare(
        &f.rules.revoke.clone(),
        f.actor,
        3,
        Input::Terminal {
            funding: token(2),
            reason,
        },
    )?;
    f.publish(f.rules.revoke.clone(), 3, p)?;
    let terminal = history(&f)?.terminal().ok_or("terminal")?.clone();
    assert_eq!(terminal.decision().operation, token(3));
    assert_eq!(terminal.decision().action, f.rules.revoke);
    assert_eq!(posted(&f, 3)?, 10000);
    // This is the real configured E4 owner leg. The fixture does not fabricate
    // a Registry sanction: production Roll's separate same-union proof is owed.
    Ok(())
}
#[test]
fn pending_funding_rechecks_actual_payer_title_and_clock_at_stage_and_take() -> Result {
    for at_take in [false, true] {
        for change in 0..4 {
            let mut f = Fixture::new()?;
            let p = f.fund(2)?;
            let mut prepared = Some(p);
            if at_take {
                f.stage(f.rules.fund.clone(), 2, prepared.take().ok_or("prepared")?)?;
            }
            let shell = f.shell;
            match change {
                0 => {
                    f.state_mut()
                        .snapshot
                        .account_mut(AccountId(3))
                        .ok_or("payer")?
                        .status = AccountStatus::Frozen
                }
                1 => {
                    f.state_mut()
                        .snapshot
                        .asset_mut(shell)
                        .ok_or("title")?
                        .revision += 1
                }
                2 => {
                    let epoch = f.world.world().next_world_timeline_epoch(2)?;
                    f.world.world_mut().append_world_timeline_epoch(epoch)?;
                }
                _ => {
                    let now = f.world.world().now();
                    f.world.world_mut().set_now(now - 1);
                }
            }
            let _ = f.world.world_mut().take_dirty();
            if at_take {
                assert!(f.world.engine_mut().take_staged_action().is_err());
            } else {
                assert!(
                    f.stage(f.rules.fund.clone(), 2, prepared.take().ok_or("prepared")?)
                        .is_err()
                );
            }
            assert!(f.state().pending_charter_histories()?.next().is_none());
            assert_eq!(
                f.state()
                    .snapshot
                    .account(AccountId(3))
                    .ok_or("payer")?
                    .held_amount,
                Amount::ZERO
            );
        }
    }
    Ok(())
}
#[test]
fn pending_post_ack_native_drift_cannot_publish_any_prepared_money_or_title() -> Result {
    let mut f = funded()?;
    let p = f.terminal(
        3,
        CharterPendingDecisionKindV1::LastClaimClosed { next_transition: 2 },
    )?;
    f.stage(f.rules.settle.clone(), 3, p)?;
    let held = f.world.engine_mut().take_staged_action()?.ok_or("taken")?;
    let (_, install) = held.into_parts();
    let shell = f.shell;
    f.state_mut()
        .snapshot
        .asset_mut(shell)
        .ok_or("title")?
        .revision += 1;
    let before = f.state().snapshot.clone();
    assert!(
        f.world
            .engine_mut()
            .install_composed_action(install)
            .is_err()
    );
    assert!(
        f.state().snapshot == before,
        "failed consuming bind must not apply any owner image"
    );
    assert!(history(&f)?.terminal().is_none());
    Ok(())
}
#[test]
fn pending_cold_recovery_refuses_missing_crossed_extra_and_disabled_native_history() -> Result {
    let f = funded()?;
    let h = history(&f)?;
    let good = f.cold(f.boot.clone())?;
    assert!(history(&good)? == h);
    for change in 0..4 {
        let mut boot = f.boot.clone();
        let data = boot.get_mut("economy").ok_or("economic boot")?;
        let hold_key = crate::api::hold_key(h.funding().hold());
        let contract_key = crate::api::contract_key(h.contract_id());
        match change {
            0 => data.kv.retain(|(k, _)| k != &hold_key),
            1 => data.kv.retain(|(k, _)| k != &contract_key),
            2 => {
                let (_, v) = data
                    .kv
                    .iter_mut()
                    .find(|(k, _)| k == &hold_key)
                    .ok_or("hold doc")?;
                let mut hold: crate::record::Hold = serde_json::from_value(v.clone())?;
                hold.holder.namespace = "foreign".into();
                *v = serde_json::to_value(hold)?;
            }
            _ => {
                let mut hold = f
                    .state()
                    .snapshot
                    .hold(h.funding().hold())
                    .ok_or("hold")?
                    .clone();
                hold.id = crate::HoldId(1000);
                hold.holder.namespace = "foreign".into();
                data.kv
                    .push((crate::api::hold_key(hold.id), serde_json::to_value(hold)?));
            }
        }
        assert!(f.cold(boot).is_err());
    }
    let mut disabled = TestWorld::with_modules(vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(crate::EconomyModule::new()),
    ]);
    let _ = disabled.take_module_ops();
    assert!(disabled.try_modules_loaded(f.boot.clone()).is_err());
    assert!(disabled.take_module_ops().is_empty());
    Ok(())
}
#[test]
fn pending_actual_journal_and_checkpoint_inverse_keep_replay_latch_and_full_envelopes() -> Result {
    let mut f = funded()?;
    let p = f.terminal(
        3,
        CharterPendingDecisionKindV1::LastClaimClosed { next_transition: 2 },
    )?;
    f.publish(f.rules.settle.clone(), 3, p)?;
    let actual = history(&f)?;
    let audit = &f.state().fold.charter_pending_receipts;
    let rows = journal_rows(&f);
    assert!(audit.checkpoint_complete(&rows));
    let retained = serde_json::to_value(audit)?;
    let mut parsed: history::CharterPendingAudit = serde_json::from_value(retained)?;
    assert!(
        parsed.get(token(2)).is_err(),
        "serialized health never grants a live latch"
    );
    parsed.finish_replay()?;
    assert!(parsed.get(token(2))? == &actual);
    assert!(!audit.checkpoint_complete(&[]));
    let mut missing = rows.clone();
    missing.retain(|r| r.ns.as_str() != NAMESPACE);
    assert!(!audit.checkpoint_complete(&missing));
    let fund = rows
        .iter()
        .find(|r| r.ns.as_str() == NAMESPACE)
        .ok_or("journal key")?
        .clone();
    let mut duplicate = rows.clone();
    duplicate.push(fund);
    assert!(!audit.checkpoint_complete(&duplicate));
    let mut txs: Vec<_> = f.state().transactions.values().cloned().collect();
    txs.sort_by_key(|t| t.journal_seq);
    let mut replay = history::CharterPendingAudit::default();
    for tx in &txs {
        replay.observe(tx);
    }
    replay.finish_replay()?;
    assert!(replay.get(token(2))? == &actual);
    let raw = mush_core::canonical_json_bytes_with_limit(
        &serde_json::to_value(&actual)?,
        MAX_PENDING_HISTORY_BYTES,
    )?;
    assert!(CharterPendingHistory::decode(&raw)? == actual);
    assert!(serde_json::to_vec(&actual.contract()?)?.len() <= MAX_PENDING_CONTRACT_BYTES);
    for tx in txs
        .iter()
        .filter(|t| t.idempotency.namespace.as_str() == NAMESPACE)
    {
        let EconomicCause::Ruleset { correlation, .. } = &tx.cause else {
            return Err("native cause".into());
        };
        assert!(correlation.len() <= MAX_PENDING_CORRELATION_BYTES);
    }
    Ok(())
}

#[test]
fn pending_funding_requires_the_actual_charter_birth_and_exact_configured_action() -> Result {
    let mut f = Fixture::new()?;
    let before_state = f.state().snapshot.clone();
    for change in 0..3 {
        let mut terms = f.terms()?;
        let mut action = f.rules.fund.clone();
        let mut actor = f.buyer;
        match change {
            0 => {
                // A self-consistent source-free request still cannot replace
                // the actual native filing birth with another lawful token.
                terms.birth = token(90);
                terms.ownership.operation = token(90);
                terms.validate()?;
            }
            1 => action = f.rules.wrong.clone(),
            _ => actor = f.actor,
        }
        assert!(
            f.prepare(
                &action,
                actor,
                20 + change,
                Input::Fund {
                    terms,
                    payer: AccountId(3),
                }
            )
            .is_err()
        );
        assert!(
            f.state().snapshot == before_state,
            "refusal changed native state"
        );
        assert!(
            f.world.take_module_ops().is_empty(),
            "refusal published native documents"
        );
    }
    let actual = f.fund(2)?;
    f.publish(f.rules.fund.clone(), 2, actual)?;
    let h = history(&f)?;
    assert_eq!(h.consent().terms.birth, token(1));
    assert!(f.cold(f.boot.clone()).is_ok());
    Ok(())
}

#[test]
fn pending_namespace_refuses_legacy_replay_and_terminal_hold_survives_real_sweep() -> Result {
    use crate::engine::{NoCustody, NoFacts, ValidateEnv};
    use crate::plan::PlanResult;
    let mut f = funded()?;
    let before = history(&f)?;
    let plan = plans::funding(before.intent())?;
    let gate = f.state().key_gate();
    let env = ValidateEnv {
        auth: &crate::AllowAll,
        custody: &NoCustody,
        facts: &NoFacts,
        now: f.world.world().now(),
        ic_now: f.world.world().game_now_unix(),
        next_hold_id: f.state().next_hold,
        spent: Default::default(),
        keys: &gate,
    };
    assert!(matches!(
        crate::engine::validate(&plan, &f.state().snapshot, &env),
        PlanResult::Refused(crate::EconomyError::ManagedContract)
    ));
    let p = f.terminal(
        3,
        CharterPendingDecisionKindV1::LastClaimClosed { next_transition: 2 },
    )?;
    f.publish(f.rules.settle.clone(), 3, p)?;
    let actual = history(&f)?;
    let at = f
        .world
        .world()
        .now()
        .checked_add(crate::HOLD_GC_AFTER_SECS + 60)
        .ok_or("gc clock")?;
    f.world.set_now(at);
    let _ = f
        .world
        .fire_due(at.checked_mul(1000).ok_or("gc milliseconds")?);
    fold_module_ops_into(&mut f.boot, f.world.take_module_ops());
    assert!(f.state().snapshot.hold(actual.funding().hold()).is_some());
    assert!(
        f.state()
            .fold
            .journal_holds
            .contains_key(&actual.funding().hold())
    );
    assert!(history(&f)? == actual);
    assert!(history(&f.cold(f.boot.clone())?)? == actual);
    Ok(())
}

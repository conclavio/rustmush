//! Real registered Economy monetary owner and cold-recovery tests. The Market
//! receipt fixture is inert: it does not prove a paid licence, seller offer,
//! claim census, Registry sanction or a production service intent.
use super::*;
use crate::record::{AccountStatus, CatalogRef, EconomicCause, HoldStatus};
use crate::registry_filing::{EconomyRequest, InitialFilingParticipant, InitialFilingPolicy};
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

use crate::managed_entitlement::{GrossRecipientCut, Recipient};
use mush_core::{ClockRef, ClockStamp};

#[derive(Clone)]
struct Rules {
    declarations: Vec<DefinitionDescriptor>,
    actions: [DefinitionRef; 5],
    wrong: DefinitionRef,
    outcome: DefinitionRef,
}
fn descriptor(kind: &str, id: &str) -> Result<DefinitionDescriptor> {
    Ok(DefinitionDescriptor::new(
        DefinitionKind::new(kind)?,
        DefinitionId::new(id)?,
        1,
        json!({"fixture":"native-paid-resale"}),
    )?)
}
fn initial_descriptor() -> Result<DefinitionDescriptor> {
    descriptor("action", "initial_filing")
}
fn initial() -> Result<DefinitionRef> {
    Ok(DefinitionRegistryBuilder::new().register(OwnerId::new("roll")?, initial_descriptor()?)?)
}
fn rules() -> Result<Rules> {
    let mut b = DefinitionRegistryBuilder::new();
    let owner = OwnerId::new("market")?;
    let mut declarations = Vec::new();
    let mut add = |kind, id| -> Result<DefinitionRef> {
        let d = descriptor(kind, id)?;
        let r = b.register(owner.clone(), d.clone())?;
        declarations.push(d);
        Ok(r)
    };
    let actions = [
        add("action", OFFER)?,
        add("action", ACCEPT)?,
        add("action", FUND)?,
        add("action", SETTLE)?,
        add("action", REFUND)?,
    ];
    let wrong = add("action", "wrong_paid_resale")?;
    let outcome = add("outcome", "paid_resale_fixture")?;
    Ok(Rules {
        declarations,
        actions,
        wrong,
        outcome,
    })
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Input {
    Accept {
        terms: PaidResaleTermsV1,
        payer: AccountId,
        claims: PaidResaleClaimsV1,
        mode: PaidResaleModeV1,
    },
    Terminal {
        acceptance: CommitToken,
        reason: PaidResaleDecisionKindV1,
    },
}
const RECEIPT_FAMILY: &str = "paid_resale_fixture_receipt";
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureReceipt {
    format: u8,
    action: DefinitionRef,
    commit: CommitToken,
    actor: ObjId,
    history: PaidResaleHistoryV1,
}
impl FixtureReceipt {
    fn validate(&self) -> std::result::Result<(), String> {
        self.history.validate().map_err(|_| "history shape")?;
        let (action, commit, actor) = match self.history.terminal() {
            None => (
                &self.history.consent().effect,
                self.history.consent().operation,
                self.history.consent().terms.buyer,
            ),
            Some(t) => (
                &t.decision().action,
                t.decision().operation,
                t.decision().actor,
            ),
        };
        if self.format != 1
            || self.actor.0 == 0
            || &self.action != action
            || self.commit != commit
            || self.actor != actor
        {
            return Err("fixture affinity".into());
        }
        Ok(())
    }
    fn verify(&self, state: &EconomyStore) -> Result {
        if state.paid_resale_history(self.history.consent().operation)? != &self.history {
            return Err("actual native output differs".into());
        }
        Ok(())
    }
}
fn receipts(
    reg: &mut Registrar<'_>,
) -> std::result::Result<StateFamilyHandle<CommitToken, FixtureReceipt>, ModError> {
    let bad = |e: &dyn std::fmt::Display| ModError::msg(e.to_string());
    reg.state_family(
        StateFamilyDescriptor::new(
            StateFamilyId::new(RECEIPT_FAMILY).map_err(|e| bad(&e))?,
            CanonicalId::new("paid_resale_fixture_receipt.key.v1").map_err(|e| bad(&e))?,
            CanonicalId::new("paid_resale_fixture_receipt.value.v1").map_err(|e| bad(&e))?,
            1,
            CanonicalId::new("paid_resale_fixture_receipt.decode.v1").map_err(|e| bad(&e))?,
            CanonicalId::new("paid_resale_fixture_receipt.validate.v1").map_err(|e| bad(&e))?,
            CanonicalId::new("paid_resale_fixture_receipt.canonical.v1").map_err(|e| bad(&e))?,
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
        move |row: &FixtureReceipt| row.validate(),
        |row: &FixtureReceipt| {
            serde_json::to_value(row).map_err(|_| "fixture receipt encoding".into())
        },
    )
}
fn prepared_receipt(
    writes: &[ReplayWrite],
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
        if namespace.as_str() != "market" {
            continue;
        }
        let receipt: FixtureReceipt =
            serde_json::from_value(value.get("value").ok_or("typed fixture value")?.clone())?;
        receipt.validate()?;
        let expected = json!({"family":{"owner":"market","id":RECEIPT_FAMILY},"key":receipt.commit.to_hex(),"schema":{"id":"paid_resale_fixture_receipt.value.v1","version":1},"value":receipt});
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
        "paid resale owner fixture refused",
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
        let input: Input = serde_json::from_value(inv.parameters().clone()).map_err(|_| error())?;
        let request = match input {
            Input::Accept {
                terms,
                payer,
                claims,
                mode,
            } => EconomyRequest::PaidResaleAcceptance(PaidResaleAcceptanceRequest {
                terms,
                payer,
                claims,
                mode,
                offer_digest: DefinitionDigest::from_bytes([44; 32]),
            }),
            Input::Terminal { acceptance, reason } => {
                let w = ctx.world().map_err(|_| error())?;
                EconomyRequest::PaidResaleTerminal(PaidResaleTerminalRequest {
                    acceptance,
                    decision: PaidResaleDecisionV1 {
                        format: 1,
                        operation: ctx.commit_token(),
                        action: self.action.clone(),
                        actor: inv.actor(),
                        at_ic: w.world_ic_now(),
                        posted_at_utc: w.now(),
                        reason,
                    },
                })
            }
        };
        let output = ctx
            .prepare_owner(&self.handle, request)
            .map_err(|_| error())?
            .paid_resale()
            .map_err(|_| error())?;
        let history = output.history().clone();
        let value = serde_json::to_value(&history).map_err(|_| error())?;
        txn.insert(
            &self.receipts,
            &ctx.commit_token(),
            FixtureReceipt {
                format: 1,
                action: self.action.clone(),
                commit: ctx.commit_token(),
                actor: inv.actor(),
                history,
            },
        )
        .map_err(|_| error())?;
        Ok(AuthoritativeActionOutput::new(
            RulesValue::new(self.outcome.clone(), value).map_err(|_| error())?,
        )
        .with_definitions(vec![self.outcome.clone()]))
    }
}
struct Unavailable;
impl ActionProvider for Unavailable {
    fn validate_invocation(&self, _: &ActionInvocation) -> std::result::Result<(), ActionError> {
        Err(error())
    }
}
// Initial filing is a real configuration prerequisite, but this fixture never
// invokes it or pretends its inert paid basis is a filing receipt.
struct Roll;
impl GameModule for Roll {
    fn name(&self) -> &'static str {
        "roll"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> std::result::Result<(), ModError> {
        let d = initial_descriptor().map_err(|e| ModError::msg(e.to_string()))?;
        let expected = d.clone();
        reg.definition_validator(
            DefinitionKind::new("action").map_err(|e| ModError::msg(e.to_string()))?,
            move |ctx: &DefinitionValidationContext<'_>| {
                let e = ctx.definition();
                if e.reference().key().id == *expected.id()
                    && e.payload() == expected.payload()
                    && e.references() == expected.references()
                {
                    Ok(())
                } else {
                    Err("fixture drift".into())
                }
            },
        )?;
        let action = reg.definition(d)?;
        reg.action_provider(action, Unavailable)
    }
}
impl GameModule for Rules {
    fn name(&self) -> &'static str {
        "market"
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
                        Err("fixture drift".into())
                    }
                },
            )?;
        }
        for d in &self.declarations {
            reg.definition(d.clone())?;
        }
        reg.action_provider(self.actions[0].clone(), Unavailable)?;
        let receipts = receipts(reg)?;
        let handle =
            reg.request_owner_participant::<InitialFilingParticipant>(self.actions[2].clone())?;
        for action in self.actions.iter().skip(1) {
            let h = if action == &self.actions[2] {
                handle.clone()
            } else {
                reg.request_owner_participant::<InitialFilingParticipant>(action.clone())?
            };
            reg.action_provider(
                action.clone(),
                Provider {
                    receipts: receipts.clone(),
                    action: action.clone(),
                    handle: h,
                    outcome: self.outcome.clone(),
                },
            )?;
        }
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
fn modules(r: &Rules, enabled: bool) -> Vec<Box<dyn GameModule>> {
    let issuer = SystemPrincipalId::new("roll.registry").expect("issuer");
    let policy = InitialFilingPolicy::new(
        issuer.clone(),
        AccountId(2),
        Amount::from_minor(100),
        CatalogRef {
            ruleset: "roll".into(),
            code: "mind_charter".into(),
        },
    )
    .expect("policy");
    let economy = crate::EconomyModule::new()
        .with_title_issuers(vec![issuer.clone()])
        .with_initial_filing(policy, initial().expect("initial"));
    let mut modules: Vec<Box<dyn GameModule>> =
        vec![Box::new(mush_org::OrgModule::new()), Box::new(Roll)];
    if enabled {
        modules.push(Box::new(r.clone()));
        modules.push(Box::new(economy.with_paid_mind_resale(
            PaidResaleConfig::new(issuer, GOD, r.actions.clone(), None).expect("resale config"),
        )));
    } else {
        modules.push(Box::new(economy));
    }
    modules
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
    fn empty_enabled(enabled: bool) -> Result<Self> {
        let rules = rules()?;
        let mut world = TestWorld::with_modules(modules(&rules, enabled));
        world.world_mut().set_now(100);
        world
            .world_mut()
            .set_game_clock(mush_core::GameClock::default());
        let actor = world.spawn_player("Resale seller");
        let buyer = world.spawn_player("Resale buyer");
        let shell = world
            .world_mut()
            .create(ObjKind::Thing, "Inert paid basis", GOD, None)?;
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
    fn empty() -> Result<Self> {
        Self::empty_enabled(true)
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
            if events.iter().any(|(_,e)|matches!(e,mush_proto::OutputEvent::Sys(t) if t.spans().iter().any(|s|s.tag.as_deref()==Some("error")))){return Err("actual money setup refused".into());}
        }
        fold_module_ops_into(&mut f.boot, f.world.take_module_ops());
        let _ = f.world.world_mut().take_dirty();
        Ok(f)
    }
    fn state(&self) -> &EconomyStore {
        self.world
            .world()
            .module_state::<EconomyStore>()
            .expect("economy")
    }
    fn state_mut(&mut self) -> &mut EconomyStore {
        self.world
            .world_mut()
            .module_state_mut::<EconomyStore>()
            .expect("economy")
    }
    fn terms(&self) -> Result<PaidResaleTermsV1> {
        let w = self.world.world();
        let digest = DefinitionDigest::from_bytes([45; 32]);
        let term = PaidResaleTermBasisV1 {
            generation: 1,
            operation: token(1),
            digest,
            clock: w.world_ic_now().clock,
            window: PaidResaleWindowV1 {
                format: 1,
                eligibility_start: 0,
                issued_at: 0,
                term_end: 1000,
                grace_end: 1100,
            },
            terms_digest: digest,
            policy_digest: digest,
        };
        let row = PaidResaleTermsV1 {
            format: 1,
            offer: token(200),
            offer_action: self.rules.actions[0].clone(),
            basis: PaidResaleBasisV1 {
                licence: 1,
                charter: self.shell,
                line: 1,
                birth: PaidBirthRefV1 {
                    route: PaidBirthRouteV1::Legacy,
                    operation: token(1),
                    digest,
                },
                holder: PaidHolderRefV1 {
                    generation: 1,
                    operation: token(1),
                    digest,
                },
                term,
                economics_digest: digest,
                royalty_chain_digest: digest,
            },
            seller: self.actor,
            buyer: self.buyer,
            instrument: InstrumentId::new("cr").ok_or("instrument")?,
            price: Amount::from_minor(503),
            registry: GrossRecipientCut {
                permille: 50,
                recipient: Some(Recipient {
                    account: AccountId(2),
                    owner: PrincipalId::System(
                        SystemPrincipalId::new("roll.registry").ok_or("issuer")?,
                    ),
                }),
            },
            venue: None,
            parents: vec![],
            remainder: Recipient {
                account: AccountId(1),
                owner: PrincipalId::Object(self.actor),
            },
            offered_at_ic: w.world_ic_now(),
            offered_at_utc: w.now(),
            expires_at_utc: w.now() + 3600,
            settle_seconds: SETTLEMENT_IC_SECONDS,
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
                    owner: OwnerId::new("market")?,
                    namespace: CanonicalId::new("paid_resale_fixture")?,
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
    fn accept(&mut self, n: u8, mode: PaidResaleModeV1) -> Result<PreparedAction> {
        let action = self.rules.actions[if mode == PaidResaleModeV1::Immediate {
            1
        } else {
            2
        }]
        .clone();
        let terms = self.terms()?;
        self.prepare(
            &action,
            self.buyer,
            n,
            Input::Accept {
                terms,
                payer: AccountId(3),
                claims: PaidResaleClaimsV1 {
                    next_transition: 4,
                    open_paid: if mode == PaidResaleModeV1::Immediate {
                        0
                    } else {
                        1
                    },
                },
                mode,
            },
        )
    }
    fn terminal(&mut self, n: u8, reason: PaidResaleDecisionKindV1) -> Result<PreparedAction> {
        let action = self.rules.actions[if reason.settles() { 3 } else { 4 }].clone();
        self.prepare(
            &action,
            GOD,
            n,
            Input::Terminal {
                acceptance: token(2),
                reason,
            },
        )
    }
    fn cold(&self, boot: Boot) -> Result<Self> {
        self.cold_enabled(boot, true)
    }
    fn cold_enabled(&self, mut boot: Boot, enabled: bool) -> Result<Self> {
        let mut f = Self::empty_enabled(enabled)?;
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
        let receipt = prepared_receipt(&writes, &action, token(n), inv.actor())?;
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
    let p = f.accept(2, PaidResaleModeV1::Pending)?;
    f.publish(f.rules.actions[2].clone(), 2, p)?;
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
fn history(f: &Fixture) -> Result<PaidResaleHistoryV1> {
    Ok(f.state().paid_resale_history(token(2))?.clone())
}

#[test]
fn resale_immediate_debits_gross_and_credits_exact_split_without_title_or_hold() -> Result {
    let mut f = Fixture::new()?;
    let p = f.accept(2, PaidResaleModeV1::Immediate)?;
    assert_eq!(
        (posted(&f, 1)?, posted(&f, 2)?, posted(&f, 3)?),
        (10000, 0, 10000)
    );
    let writes = f.publish(f.rules.actions[1].clone(), 2, p)?;
    assert!(
        writes
            .iter()
            .any(|w| matches!(w, ReplayWrite::LedgerChunkSet { .. }))
    );
    assert_eq!(
        (posted(&f, 1)?, posted(&f, 2)?, posted(&f, 3)?),
        (10478, 25, 9497)
    );
    let h = history(&f)?;
    assert_eq!(h.outcome(), PaidResaleOutcome::Settled);
    assert!(h.acceptance().hold().is_none() && h.terminal().is_none());
    assert!(f.state().snapshot.asset(f.shell).is_none());
    let tx = f
        .state()
        .transactions
        .get(&h.acceptance().transaction())
        .ok_or("actual transaction")?;
    assert!(
        tx.asset_effects.is_empty() && tx.hold_effects.is_empty() && tx.custody_effects.is_empty()
    );
    assert!(history(&f.cold(f.boot.clone())?)? == h);
    Ok(())
}

#[test]
fn resale_pending_holds_gross_then_native_last_close_pays_and_replays() -> Result {
    let mut f = funded()?;
    let before = history(&f)?;
    let hold = before.acceptance().hold().ok_or("held id")?;
    assert_eq!(
        (posted(&f, 1)?, posted(&f, 2)?, posted(&f, 3)?),
        (10000, 0, 10000)
    );
    assert_eq!(
        f.state().snapshot.hold(hold).ok_or("held")?.status,
        HoldStatus::Held
    );
    let p = f.terminal(
        3,
        PaidResaleDecisionKindV1::LastClaimClosed { next_transition: 5 },
    )?;
    f.publish(f.rules.actions[3].clone(), 3, p)?;
    let after = history(&f)?;
    assert_eq!(after.consent(), before.consent());
    assert_eq!(
        (posted(&f, 1)?, posted(&f, 2)?, posted(&f, 3)?),
        (10478, 25, 9497)
    );
    assert!(matches!(
        f.state().snapshot.hold(hold).ok_or("terminal hold")?.status,
        HoldStatus::Consumed { .. }
    ));
    assert!(history(&f.cold(f.boot.clone())?)? == after);
    assert!(
        f.terminal(
            4,
            PaidResaleDecisionKindV1::LastClaimClosed { next_transition: 6 }
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn resale_refund_uses_retained_money_when_buyer_unavailable_or_clock_changed() -> Result {
    for changed_clock in [false, true] {
        let mut f = funded()?;
        let reason = if changed_clock {
            let epoch = f.world.world().next_world_timeline_epoch(2)?;
            f.world.world_mut().append_world_timeline_epoch(epoch)?;
            PaidResaleDecisionKindV1::ClockChanged {}
        } else {
            let buyer = f.buyer;
            f.world
                .world_mut()
                .get_mut(buyer)
                .ok_or("buyer")?
                .flags
                .insert(mush_core::Flags::GOING);
            let _ = f.world.world_mut().take_dirty();
            PaidResaleDecisionKindV1::RecipientUnavailable {}
        };
        assert!(
            f.terminal(
                3,
                PaidResaleDecisionKindV1::LastClaimClosed { next_transition: 5 }
            )
            .is_err()
        );
        let p = f.terminal(3, reason)?;
        f.publish(f.rules.actions[4].clone(), 3, p)?;
        let h = history(&f)?;
        assert_eq!(h.outcome(), PaidResaleOutcome::Refunded);
        assert_eq!(
            (posted(&f, 1)?, posted(&f, 2)?, posted(&f, 3)?),
            (10000, 0, 10000)
        );
        assert_eq!(
            f.state()
                .snapshot
                .hold(h.acceptance().hold().ok_or("id")?)
                .ok_or("hold")?
                .status,
            HoldStatus::Released
        );
        assert!(history(&f.cold(f.boot.clone())?)? == h);
    }
    Ok(())
}

#[test]
fn resale_deadline_is_actual_and_lapsed_property_keeps_immutable_dates() -> Result {
    let mut f = funded()?;
    let reason = PaidResaleDecisionKindV1::DeadlineReached {
        next_transition: 4,
        open_paid: 1,
    };
    assert!(f.terminal(3, reason.clone()).is_err());
    f.world.world_mut().set_now(1000);
    let p = f.terminal(3, reason)?;
    f.publish(f.rules.actions[3].clone(), 3, p)?;
    assert_eq!(history(&f)?.outcome(), PaidResaleOutcome::Settled);
    let mut f = Fixture::new()?;
    f.world.world_mut().set_now(1200);
    let terms = f.terms()?;
    let original = terms.basis.term.clone();
    let action = f.rules.actions[1].clone();
    let p = f.prepare(
        &action,
        f.buyer,
        2,
        Input::Accept {
            terms,
            payer: AccountId(3),
            claims: PaidResaleClaimsV1 {
                next_transition: 4,
                open_paid: 0,
            },
            mode: PaidResaleModeV1::Immediate,
        },
    )?;
    f.publish(action, 2, p)?;
    let h = history(&f)?;
    assert_eq!(h.consent().terms.basis.term, original);
    assert!(h.consent().accepted_at_ic.at >= original.window.grace_end);
    assert!(history(&f.cold(f.boot.clone())?)? == h);
    Ok(())
}

#[test]
fn resale_actor_action_offer_reuse_and_false_refund_reasons_refuse() -> Result {
    let mut f = Fixture::new()?;
    let input = || Input::Accept {
        terms: f.terms().expect("terms"),
        payer: AccountId(3),
        claims: PaidResaleClaimsV1 {
            next_transition: 4,
            open_paid: 1,
        },
        mode: PaidResaleModeV1::Pending,
    };
    let wrong_actor = input();
    let wrong_action = input();
    assert!(
        f.prepare(&f.rules.actions[2].clone(), f.actor, 2, wrong_actor)
            .is_err()
    );
    assert!(
        f.prepare(&f.rules.wrong.clone(), f.buyer, 2, wrong_action)
            .is_err()
    );
    let p = f.accept(2, PaidResaleModeV1::Pending)?;
    f.publish(f.rules.actions[2].clone(), 2, p)?;
    for reason in [
        PaidResaleDecisionKindV1::ClockChanged {},
        PaidResaleDecisionKindV1::RecipientUnavailable {},
        PaidResaleDecisionKindV1::MonetaryBindingChanged {},
    ] {
        assert!(f.terminal(3, reason).is_err());
    }
    let p = f.terminal(
        3,
        PaidResaleDecisionKindV1::LastClaimClosed { next_transition: 5 },
    )?;
    f.publish(f.rules.actions[3].clone(), 3, p)?;
    assert!(
        f.accept(4, PaidResaleModeV1::Immediate).is_err(),
        "used offer remains consumed"
    );
    Ok(())
}

#[test]
fn resale_money_preimages_and_live_actor_repeat_at_stage_take_and_bind() -> Result {
    for boundary in 0..3 {
        for drift in 0..2 {
            let mut f = Fixture::new()?;
            let p = f.accept(2, PaidResaleModeV1::Immediate)?;
            let action = f.rules.actions[1].clone();
            let held = if boundary == 2 {
                f.stage(action.clone(), 2, p)?;
                Some(f.world.engine_mut().take_staged_action()?.ok_or("held")?)
            } else {
                if boundary == 1 {
                    f.stage(action.clone(), 2, p)?;
                } else {
                    if drift == 0 {
                        let mut a = f
                            .state()
                            .snapshot
                            .account(AccountId(1))
                            .ok_or("seller account")?
                            .clone();
                        a.status = AccountStatus::Frozen;
                        f.state_mut().snapshot.put_account(a);
                    } else {
                        let buyer = f.buyer;
                        f.world
                            .world_mut()
                            .get_mut(buyer)
                            .ok_or("buyer")?
                            .flags
                            .insert(mush_core::Flags::GOING);
                        let _ = f.world.world_mut().take_dirty();
                    }
                    assert!(f.stage(action, 2, p).is_err());
                    assert_eq!(posted(&f, 3)?, 10000);
                    continue;
                }
                None
            };
            if drift == 0 {
                let mut a = f
                    .state()
                    .snapshot
                    .account(AccountId(1))
                    .ok_or("seller account")?
                    .clone();
                a.status = AccountStatus::Frozen;
                f.state_mut().snapshot.put_account(a);
            } else if boundary < 2 {
                let buyer = f.buyer;
                f.world
                    .world_mut()
                    .get_mut(buyer)
                    .ok_or("buyer")?
                    .flags
                    .insert(mush_core::Flags::GOING);
                let _ = f.world.world_mut().take_dirty();
            } else {
                // After custody, only local mutable owner images may invalidate
                // bind; authority has already been taken by the real engine.
                f.state_mut().next_transaction += 1;
            }
            if let Some(held) = held {
                let (_, installation) = held.into_parts();
                assert!(
                    f.world
                        .engine_mut()
                        .install_composed_action(installation)
                        .is_err()
                );
            } else {
                assert!(f.world.engine_mut().take_staged_action().is_err());
            }
            assert_eq!(posted(&f, 3)?, 10000);
            assert_eq!(f.state().paid_resale_histories()?.len(), 0);
        }
    }
    Ok(())
}

#[test]
fn resale_cold_inventory_refuses_missing_or_crossed_native_documents() -> Result {
    let f = funded()?;
    let h = history(&f)?;
    for which in 0..5 {
        let mut boot = f.boot.clone();
        let economy = boot.get_mut("economy").ok_or("economy boot")?;
        let contract_key = crate::api::contract_key(h.contract_id());
        let hold_key = crate::api::hold_key(h.acceptance().hold().ok_or("hold")?);
        match which {
            0 => economy.kv.retain(|(key, _)| key != &contract_key),
            1 => economy.kv.retain(|(key, _)| key != &hold_key),
            2 => {
                let (_, v) = economy
                    .kv
                    .iter_mut()
                    .find(|(key, _)| key == &contract_key)
                    .ok_or("contract")?;
                v["name"] = json!("crossed");
            }
            3 => {
                let (_, v) = economy
                    .kv
                    .iter_mut()
                    .find(|(key, _)| key == &hold_key)
                    .ok_or("hold")?;
                v["amount"] = serde_json::to_value(Amount::from_minor(502))?;
            }
            _ => {
                let (_, v) = economy
                    .kv
                    .iter_mut()
                    .find(|(key, _)| key == &hold_key)
                    .ok_or("hold")?;
                v["holder"]["namespace"] = json!("foreign.owner");
            }
        }
        assert!(
            f.cold(boot).is_err(),
            "native inverse audit must refuse corrupted document"
        );
    }
    assert!(history(&f.cold(f.boot.clone())?)? == h);
    Ok(())
}

#[test]
fn resale_sparse_install_matches_full_fold_and_checkpoint_requires_exact_history() -> Result {
    let mut f = Fixture::new()?;
    let before = f.state().fold.clone();
    let p = f.accept(2, PaidResaleModeV1::Pending)?;
    f.publish(f.rules.actions[2].clone(), 2, p)?;
    let a = history(&f)?.acceptance().transaction();
    let tx = f
        .state()
        .transactions
        .get(&a)
        .ok_or("actual funding transaction")?
        .clone();
    let mut full = before;
    full.fold_entry(tx.journal_seq, &tx);
    assert!(
        serde_json::to_value(&full)? == serde_json::to_value(&f.state().fold)?,
        "sparse and replay folds differ"
    );
    let index: Vec<_> = f
        .state()
        .transactions
        .values()
        .map(|tx| crate::checkpoint::IdemRow {
            ns: tx.idempotency.namespace.clone(),
            key: tx.idempotency.key.clone(),
            tx: tx.id,
            seq: tx.journal_seq,
            posted_at: tx.posted_at,
            plan_hash: tx.plan_hash.iter().map(|b| format!("{b:02x}")).collect(),
        })
        .collect();
    assert!(
        f.state()
            .fold
            .paid_resale_receipts
            .checkpoint_complete(&index)
    );
    let mut missing = index.clone();
    missing.retain(|r| r.ns.as_str() != NAMESPACE);
    assert!(
        !f.state()
            .fold
            .paid_resale_receipts
            .checkpoint_complete(&missing)
    );
    let mut duplicate = index;
    let row = duplicate
        .iter()
        .find(|r| r.ns.as_str() == NAMESPACE)
        .ok_or("row")?
        .clone();
    duplicate.push(row);
    assert!(
        !f.state()
            .fold
            .paid_resale_receipts
            .checkpoint_complete(&duplicate)
    );
    Ok(())
}

#[test]
fn resale_namespace_blocks_legacy_posts_and_real_sweep_retains_terminal_hold() -> Result {
    use crate::engine::{NoCustody, NoFacts, ValidateEnv};
    use crate::plan::PlanResult;
    let mut f = funded()?;
    let h = history(&f)?;
    {
        let plan = plans::acceptance(h.intent())?;
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
    }
    let p = f.terminal(
        3,
        PaidResaleDecisionKindV1::LastClaimClosed { next_transition: 5 },
    )?;
    f.publish(f.rules.actions[3].clone(), 3, p)?;
    let actual = history(&f)?;
    let hold = actual.acceptance().hold().ok_or("hold")?;
    let at = f
        .world
        .world()
        .now()
        .checked_add(crate::HOLD_GC_AFTER_SECS + 60)
        .ok_or("gc clock")?;
    f.world.set_now(at);
    let _ = f.world.fire_due(at.checked_mul(1000).ok_or("gc millis")?);
    fold_module_ops_into(&mut f.boot, f.world.take_module_ops());
    assert!(f.state().snapshot.hold(hold).is_some());
    assert!(f.state().fold.journal_holds.contains_key(&hold));
    assert!(history(&f.cold(f.boot.clone())?)? == actual);
    Ok(())
}

#[test]
fn resale_disabled_boot_refuses_retained_native_history_without_fixture_rows() -> Result {
    let f = funded()?;
    let mut boot = f.boot.clone();
    // Remove the inert fixture family's namespace so refusal must come from
    // native retained history, not an unavailable typed fixture registration.
    boot.remove("market");
    assert!(f.cold_enabled(boot, false).is_err());
    let empty = Fixture::new()?;
    assert!(empty.cold_enabled(empty.boot.clone(), false).is_ok());
    Ok(())
}

#[test]
fn resale_six_real_payees_include_separate_gross_buyer_royalty_and_parent_title_fences() -> Result {
    use crate::managed_entitlement::{RoyaltyRecipientCut, RoyaltyTitleRecipient};
    for invalidate_parent in [false, true] {
        let mut f = Fixture::new()?;
        let first = f.world.spawn_player("First royalty recipient");
        let second = f.world.spawn_player("Second royalty recipient");
        for command in [
            "@account/system resale.venue=cr".to_owned(),
            format!("@account/open #{}=cr", first.0),
            format!("@account/open #{}=cr", second.0),
        ] {
            let events = f.world.run(GOD, &command);
            if events.iter().any(|(_,e)|matches!(e,mush_proto::OutputEvent::Sys(t) if t.spans().iter().any(|s|s.tag.as_deref()==Some("error")))) {return Err("actual payee setup refused".into());}
        }
        let mut parents = Vec::new();
        for (i, (owner, account, rate)) in [(first, 5, 30), (second, 6, 30), (f.buyer, 3, 40)]
            .into_iter()
            .enumerate()
        {
            let object = f.world.world_mut().create(
                ObjKind::Thing,
                &format!("Native royalty title {i}"),
                GOD,
                None,
            )?;
            let events = f
                .world
                .run(GOD, &format!("@asset/title #{}=#{}", object.0, owner.0));
            if events.iter().any(|(_,e)|matches!(e,mush_proto::OutputEvent::Sys(t) if t.spans().iter().any(|s|s.tag.as_deref()==Some("error")))) {return Err("actual title setup refused".into());}
            let title = f.state().snapshot.asset(object).ok_or("actual title")?;
            parents.push(RoyaltyRecipientCut {
                charter: object,
                permille: rate,
                title: Some(RoyaltyTitleRecipient {
                    title_revision: title.revision,
                    recipient: Recipient {
                        account: AccountId(account),
                        owner: PrincipalId::Object(owner),
                    },
                }),
            });
        }
        fold_module_ops_into(&mut f.boot, f.world.take_module_ops());
        let _ = f.world.world_mut().take_dirty();
        let mut terms = f.terms()?;
        terms.venue = Some(GrossRecipientCut {
            permille: 50,
            recipient: Some(Recipient {
                account: AccountId(4),
                owner: PrincipalId::System(SystemPrincipalId::new("resale.venue").ok_or("venue")?),
            }),
        });
        terms.parents = parents;
        let title_id = terms.parents[0].charter;
        let p = f.prepare(
            &f.rules.actions[2].clone(),
            f.buyer,
            2,
            Input::Accept {
                terms,
                payer: AccountId(3),
                claims: PaidResaleClaimsV1 {
                    next_transition: 4,
                    open_paid: 1,
                },
                mode: PaidResaleModeV1::Pending,
            },
        )?;
        f.publish(f.rules.actions[2].clone(), 2, p)?;
        let h = history(&f)?;
        assert_eq!(
            f.state()
                .snapshot
                .hold(h.acceptance().hold().ok_or("id")?)
                .ok_or("hold")?
                .amount
                .minor(),
            503,
            "buyer royalty must not net the gross hold"
        );
        if invalidate_parent {
            let mut title = f.state().snapshot.asset(title_id).ok_or("title")?.clone();
            title.revision += 1;
            f.state_mut().snapshot.put_asset(title);
            assert!(
                f.terminal(
                    3,
                    PaidResaleDecisionKindV1::LastClaimClosed { next_transition: 5 }
                )
                .is_err()
            );
            let p = f.terminal(3, PaidResaleDecisionKindV1::MonetaryBindingChanged {})?;
            f.publish(f.rules.actions[4].clone(), 3, p)?;
            assert_eq!(posted(&f, 3)?, 10000);
        } else {
            let p = f.terminal(
                3,
                PaidResaleDecisionKindV1::LastClaimClosed { next_transition: 5 },
            )?;
            f.publish(f.rules.actions[3].clone(), 3, p)?;
            assert_eq!(
                (
                    posted(&f, 1)?,
                    posted(&f, 2)?,
                    posted(&f, 3)?,
                    posted(&f, 4)?,
                    posted(&f, 5)?,
                    posted(&f, 6)?
                ),
                (10403, 25, 9517, 25, 15, 15)
            );
            let after = history(&f)?;
            let tx = f
                .state()
                .transactions
                .get(&after.terminal().ok_or("terminal")?.transaction())
                .ok_or("transaction")?;
            assert_eq!(tx.lines.len(), 7);
            let buyer_lines: Vec<_> = tx
                .lines
                .iter()
                .filter(|l| l.account == AccountId(3))
                .map(|l| l.delta.minor())
                .collect();
            assert_eq!(buyer_lines.len(), 2);
            assert!(buyer_lines.contains(&-503) && buyer_lines.contains(&20));
        }
    }
    Ok(())
}

mod holder_renewal;

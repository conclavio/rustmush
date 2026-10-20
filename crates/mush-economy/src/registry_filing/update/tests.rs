#![cfg(test)]

//! Actual registered owner preparation and native fee posting. The Roll fixture
//! supplies only action definitions; this is not a Registry/certificate fixture.
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
    derived: Option<DefinitionRef>,
    major: Option<DefinitionRef>,
    house_initial: Option<DefinitionRef>,
    house_update: Option<DefinitionRef>,
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
        derived: None,
        major: None,
        house_initial: None,
        house_update: None,
        wrong,
        outcome,
    })
}
fn derived_rules() -> Result<Rules> {
    let mut r = rules()?;
    let d = descriptor("action", DERIVED_ACTION_ID)?;
    let actual = DefinitionRegistryBuilder::new().register(OwnerId::new("roll")?, d.clone())?;
    r.declarations.push(d);
    r.derived = Some(actual);
    Ok(r)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    charter: ObjId,
    payer: AccountId,
    title_revision: u64,
    #[serde(default)]
    organization: Option<mush_org::OrgId>,
    #[serde(default)]
    scope: Option<mush_org::OrgScopeId>,
}
// Strict test-only typed evidence derived from each actual prepared Economy
// output. It is not a production Roll filing/update record or an authority.
const RECEIPT_FAMILY: &str = "fee_update_fixture_receipt";
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "subject",
    rename_all = "snake_case",
    deny_unknown_fields
)]
enum FixtureEffect {
    Initial(super::super::receipt::InitialFilingAuditSubject),
    Update(UpdateAuditSubject),
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
        derived: Option<&DefinitionRef>,
        major: Option<&DefinitionRef>,
        house_initial: Option<&DefinitionRef>,
        house_update: Option<&DefinitionRef>,
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
            FixtureEffect::Initial(s)
                if house_initial == Some(&self.action) && s.commit() == self.commit =>
            {
                if s.house().is_none_or(|h| h.curator() != self.actor) {
                    return Err("House curator differs".into());
                }
            }
            FixtureEffect::Update(s)
                if ((&self.action == update && s.kind.is_none())
                    || (derived == Some(&self.action)
                        && s.kind == Some(UpdateKind::DependentV1))
                    || (major == Some(&self.action)
                        && s.kind == Some(UpdateKind::OriginalMajorV1))
                    || (house_update == Some(&self.action)
                        && s.kind == Some(UpdateKind::HouseCompatibleV1)))
                    && s.commit == self.commit
                    && s.actor == self.actor =>
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
            FixtureEffect::Update(subject) => state.update_receipt_audit().verify(subject)?,
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
    let derived = rules.derived.clone();
    let major = rules.major.clone();
    let house_initial = rules.house_initial.clone();
    let house_update = rules.house_update.clone();
    reg.state_family(
        StateFamilyDescriptor::new(
            StateFamilyId::new(RECEIPT_FAMILY).map_err(|e| bad(&e))?,
            CanonicalId::new("fee_update_fixture_receipt.key.v1").map_err(|e| bad(&e))?,
            CanonicalId::new("fee_update_fixture_receipt.value.v1").map_err(|e| bad(&e))?,
            1,
            CanonicalId::new("fee_update_fixture_receipt.decode.v1").map_err(|e| bad(&e))?,
            CanonicalId::new("fee_update_fixture_receipt.validate.v1").map_err(|e| bad(&e))?,
            CanonicalId::new("fee_update_fixture_receipt.canonical.v1").map_err(|e| bad(&e))?,
        )
        .map_err(|e| bad(&e))?
        .with_limits(32, 4096)
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
        move |row: &FixtureReceipt| {
            row.validate(
                &initial,
                &update,
                derived.as_ref(),
                major.as_ref(),
                house_initial.as_ref(),
                house_update.as_ref(),
            )
        },
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
        receipt.validate(
            &rules.initial,
            &rules.update,
            rules.derived.as_ref(),
            rules.major.as_ref(),
            rules.house_initial.as_ref(),
            rules.house_update.as_ref(),
        )?;
        let expected = json!({"family":{"owner":"roll","id":RECEIPT_FAMILY},"key":receipt.commit.to_hex(),"schema":{"id":"fee_update_fixture_receipt.value.v1","version":1},"value":receipt});
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
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> std::result::Result<AuthoritativeActionOutput, ActionError> {
        let r: Input = serde_json::from_value(inv.parameters().clone()).map_err(|_| error())?;
        let request = if self.action.key().id.as_str() == HOUSE_ACTION_ID {
            EconomyRequest::HouseCompatibleUpdate(RegistryHouseCompatibleUpdate {
                charter: r.charter,
                payer: r.payer,
                title_revision: r.title_revision,
                organization: r.organization.ok_or_else(error)?,
                scope: r.scope.ok_or_else(error)?,
            })
        } else if self.action.key().id.as_str() == HOUSE_FILING_ACTION {
            EconomyRequest::HouseInitialFiling(RegistryHouseFiling {
                shell: r.charter,
                payer: r.payer,
                organization: r.organization.ok_or_else(error)?,
                scope: r.scope.ok_or_else(error)?,
            })
        } else if self.initial {
            EconomyRequest::InitialFiling(RegistryInitialFiling {
                shell: r.charter,
                payer: r.payer,
                owner: PrincipalId::Object(inv.actor()),
                scope: None,
            })
        } else if self.action.key().id.as_str() == MAJOR_ACTION_ID {
            EconomyRequest::OriginalMajor(RegistryCompatibleUpdate {
                charter: r.charter,
                payer: r.payer,
                title_revision: r.title_revision,
            })
        } else if self.action.key().id.as_str() == DERIVED_ACTION_ID {
            EconomyRequest::DerivedCompatibleUpdate(RegistryCompatibleUpdate {
                charter: r.charter,
                payer: r.payer,
                title_revision: r.title_revision,
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
        let effect = if self.initial {
            let output = output.initial_filing().map_err(|_| error())?;
            let title = output.title();
            let subject = if let Some(house) = output.house() {
                super::super::receipt::InitialFilingAuditSubject::new_house(
                    output.commit_token(),
                    output.transaction(),
                    output.journal_seq(),
                    title.object,
                    title.revision,
                    output.payer(),
                    output.sink(),
                    output.fee(),
                    output.issuer().clone(),
                    title.catalog.clone().ok_or_else(error)?,
                    house.clone(),
                )
                .map_err(|_| error())?
            } else {
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
                .map_err(|_| error())?
            };
            FixtureEffect::Initial(subject)
        } else {
            FixtureEffect::Update(
                output
                    .compatible_update()
                    .map_err(|_| error())?
                    .subject()
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
        let mut providers = vec![
            (self.initial.clone(), initial, true),
            (self.update.clone(), update.clone(), false),
            (self.wrong.clone(), update, false),
        ];
        for action in self
            .derived
            .iter()
            .chain(self.major.iter())
            .chain(self.house_update.iter())
        {
            let handle =
                reg.request_owner_participant::<InitialFilingParticipant>(action.clone())?;
            providers.push((action.clone(), handle, false));
        }
        if let Some(action) = &self.house_initial {
            let handle =
                reg.request_owner_participant::<InitialFilingParticipant>(action.clone())?;
            providers.push((action.clone(), handle, true));
        }
        for (action, handle, is_initial) in providers {
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
    let mut economy = crate::EconomyModule::new()
        .with_title_issuers(vec![
            SystemPrincipalId::new("roll.registry").expect("issuer"),
        ])
        .with_initial_filing(policy(fee), r.initial.clone())
        .with_compatible_mind_updates(r.update.clone());
    if let Some(action) = &r.derived {
        economy = economy.with_derived_mind_updates(action.clone());
    }
    if let Some(action) = &r.major {
        economy = economy.with_original_mind_majors(action.clone());
    }
    if let Some(action) = &r.house_initial {
        economy = economy.with_house_initial_filing(action.clone());
    }
    if let Some(action) = &r.house_update {
        economy = economy.with_house_mind_updates(action.clone());
    }
    let org: Box<dyn GameModule> = if r.house_initial.is_some() {
        Box::new(house::PolicyOrg(mush_org::OrgModule::new()))
    } else {
        Box::new(mush_org::OrgModule::new())
    };
    vec![org, Box::new(r.clone()), Box::new(economy)]
}
struct Fixture {
    world: TestWorld,
    rules: Rules,
    actor: ObjId,
    shell: ObjId,
    boot: Boot,
    staged: Option<FixtureReceipt>,
}
impl Fixture {
    fn empty() -> Result<Self> {
        Self::empty_rules(rules()?)
    }
    fn empty_rules(rules: Rules) -> Result<Self> {
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
            staged: None,
        })
    }
    fn new() -> Result<Self> {
        Self::initialize(Self::empty()?)
    }
    fn new_derived() -> Result<Self> {
        Self::initialize(Self::empty_rules(derived_rules()?)?)
    }
    fn initialize(mut f: Self) -> Result<Self> {
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
        let parameters = json!({"charter":self.shell,"payer":AccountId(1),"title_revision":self.state().snapshot.asset(self.shell).map_or(0,|a|a.revision)});
        self.prepare_parameters(action, actor, n, parameters)
    }
    fn prepare_parameters(
        &mut self,
        action: &DefinitionRef,
        actor: ObjId,
        n: u8,
        parameters: Value,
    ) -> Result<PreparedAction> {
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
            parameters,
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
    assert_eq!(
        writes.len(),
        6,
        "five unchanged Economy writes plus one actual typed fixture receipt"
    );
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
        let plan = plan(None, &policy(100), f.actor, token(2), &request);
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

#[test]
fn derived_fee_has_distinct_native_history_and_preserves_original_receipt_bytes() -> Result {
    let mut f = Fixture::new_derived()?;
    let title = f.state().snapshot.asset(f.shell).ok_or("title")?.clone();
    let initial = serde_json::to_value(f.state().filing_receipt_audit())?;
    let original_action = f.rules.update.clone();
    let original = f.prepare(&original_action, f.actor, 2)?;
    f.publish(original_action, 2, original)?;
    let original_subject = f
        .state()
        .update_receipt_audit()
        .receipts()?
        .next()
        .ok_or("original receipt")?
        .clone();
    let original_json = serde_json::to_value(&original_subject)?;
    assert!(original_json.get("kind").is_none());
    assert_eq!(
        serde_json::from_value::<UpdateAuditSubject>(original_json.clone())?,
        original_subject
    );

    let action = f.rules.derived.clone().ok_or("derived action")?;
    let prepared = f.prepare(&action, f.actor, 3)?;
    let before = f.state().snapshot.clone();
    let writes = f.stage(action.clone(), 3, prepared)?;
    assert_eq!(f.state().snapshot, before);
    let prepared = prepared_receipt(&writes, &f.rules, &action, token(3), f.actor)?;
    let FixtureEffect::Update(subject) = prepared.effect else {
        return Err("actual derived output".into());
    };
    assert_eq!(subject.kind, Some(UpdateKind::DependentV1));
    assert_eq!(subject.commit, token(3));
    f.take_install()?;
    f.state().update_receipt_audit().verify(&subject)?;
    let tx = f
        .state()
        .transaction(subject.transaction)
        .ok_or("derived native transaction")?;
    assert_eq!(
        tx.idempotency.namespace.as_str(),
        receipt::DERIVED_NAMESPACE
    );
    assert_eq!(receipt::subject_of(tx)?, subject);
    assert_eq!(
        crate::contract::managed::require_legacy_transaction(tx, &f.state().snapshot),
        Err(crate::EconomyError::ManagedContract)
    );
    assert_eq!(f.state().snapshot.asset(f.shell), Some(&title));
    assert_eq!(
        serde_json::to_value(f.state().filing_receipt_audit())?,
        initial
    );
    assert_eq!(
        serde_json::to_value(
            f.state()
                .update_receipt_audit()
                .receipts()?
                .find(|s| s.commit == token(2))
                .ok_or("retained original")?
        )?,
        original_json
    );
    assert_eq!(
        f.state()
            .snapshot
            .account(AccountId(1))
            .ok_or("payer")?
            .posted_balance,
        Amount::from_minor(9700)
    );
    assert_eq!(
        f.state()
            .snapshot
            .account(AccountId(2))
            .ok_or("sink")?
            .posted_balance,
        Amount::from_minor(300)
    );

    let mut replay = UpdateReceiptAudit::default();
    let mut transactions: Vec<_> = f.state().transactions.values().collect();
    transactions.sort_by_key(|tx| tx.journal_seq);
    for tx in transactions {
        replay.observe(tx);
    }
    replay.finish_replay()?;
    assert_eq!(replay, *f.state().update_receipt_audit());
    let mut crossed = subject.clone();
    crossed.kind = None;
    assert!(replay.verify(&crossed).is_err());
    for kind in [Value::Null, json!("original"), json!({"dependent_v1":{}})] {
        let mut malformed = serde_json::to_value(&subject)?;
        malformed["kind"] = kind;
        assert!(serde_json::from_value::<UpdateAuditSubject>(malformed).is_err());
    }
    // A caller-supplied ordinary plan cannot use the new namespace, even with
    // an authorizer that would permit every monetary line.
    struct Permit;
    impl AuthEval for Permit {
        fn authorize(&self, _: &PrincipalId, _: &Account, _: Amount) -> bool {
            true
        }
    }
    let request = RegistryCompatibleUpdate {
        charter: f.shell,
        payer: AccountId(1),
        title_revision: title.revision,
    };
    let ordinary = plan(
        Some(UpdateKind::DependentV1),
        &policy(100),
        f.actor,
        token(4),
        &request,
    );
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
        crate::engine::validate(&ordinary, &f.state().snapshot, &env),
        PlanResult::Refused(crate::EconomyError::ManagedContract)
    ));
    Ok(())
}

#[test]
fn derived_fee_cold_boot_requires_its_exact_enabled_protocol_and_checkpoint_namespace() -> Result {
    let mut f = Fixture::new_derived()?;
    let original = f.update(2)?;
    f.publish(f.rules.update.clone(), 2, original)?;
    let action = f.rules.derived.clone().ok_or("derived action")?;
    let prepared = f.prepare(&action, f.actor, 3)?;
    f.publish(action, 3, prepared)?;
    let mut cold = Fixture::empty_rules(derived_rules()?)?;
    cold.world.try_modules_loaded(f.boot.clone())?;
    assert_eq!(cold.state().snapshot, f.state().snapshot);
    assert_eq!(
        cold.state().update_receipt_audit(),
        f.state().update_receipt_audit()
    );
    assert_eq!(cold.state().filing_receipt_audit().receipts()?.len(), 1);
    let mut old_only = Fixture::empty()?;
    let error = old_only
        .world
        .try_modules_loaded(f.boot.clone())
        .err()
        .ok_or("derived history accepted with original-only configuration")?;
    assert!(
        error
            .to_string()
            .contains("compatible-update protocol requires exact paired composition")
    );

    let audit = f.state().update_receipt_audit();
    let rows: Vec<_> = audit
        .receipts()?
        .map(|subject| crate::checkpoint::IdemRow {
            ns: receipt::namespace(subject.kind).into(),
            key: subject.commit.to_hex().into(),
            plan_hash: subject
                .plan_hash
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect(),
            tx: subject.transaction,
            posted_at: subject.posted_at_utc,
            seq: subject.journal_seq,
        })
        .collect();
    assert!(audit.checkpoint_complete(&rows));
    for i in 0..rows.len() {
        let mut missing = rows.clone();
        missing.remove(i);
        assert!(!audit.checkpoint_complete(&missing));
        let mut crossed = rows.clone();
        let row = crossed.get_mut(i).ok_or("checkpoint row")?;
        row.ns = if row.ns.as_str() == receipt::NAMESPACE {
            receipt::DERIVED_NAMESPACE
        } else {
            receipt::NAMESPACE
        }
        .into();
        assert!(!audit.checkpoint_complete(&crossed));
    }
    assert!(audit.require_enabled(true, false).is_err());
    assert!(audit.require_enabled(false, true).is_err());
    audit.require_enabled(true, true)?;
    Ok(())
}

#[test]
fn derived_fee_repeats_live_actor_title_and_account_checks_at_every_custody_boundary() -> Result {
    for phase in 0..3 {
        for drift in 0..3 {
            let mut f = Fixture::new_derived()?;
            let action = f.rules.derived.clone().ok_or("derived action")?;
            let mut prepared = if phase > 0 {
                Some(f.prepare(&action, f.actor, 2)?)
            } else {
                None
            };
            if phase == 2 {
                f.stage(action.clone(), 2, prepared.take().ok_or("prepared")?)?;
            }
            let shell = f.shell;
            let actor = f.actor;
            match drift {
                0 => {
                    f.state_mut()
                        .snapshot
                        .asset_mut(shell)
                        .ok_or("title")?
                        .frozen = true
                }
                1 => {
                    f.state_mut()
                        .snapshot
                        .account_mut(AccountId(1))
                        .ok_or("payer")?
                        .owner = PrincipalId::Object(GOD)
                }
                _ => {
                    f.world
                        .world_mut()
                        .get_mut(actor)
                        .ok_or("actor")?
                        .flags
                        .insert(mush_core::Flags::GOING);
                    let _ = f.world.world_mut().take_dirty();
                }
            }
            let before = f.state().snapshot.clone();
            let next = f.state().next_transaction;
            match phase {
                0 => assert!(f.prepare(&action, actor, 2).is_err()),
                1 => assert!(
                    f.stage(action, 2, prepared.take().ok_or("prepared")?)
                        .is_err()
                ),
                _ => assert!(f.world.engine_mut().take_staged_action().is_err()),
            }
            assert_eq!(f.state().snapshot, before);
            assert_eq!(f.state().next_transaction, next);
            assert_eq!(f.state().update_receipt_audit().receipts()?.len(), 0);
        }
    }
    Ok(())
}

#[test]
fn derived_fee_registration_requires_initial_policy_positive_fee_and_exact_action() -> Result {
    let r = derived_rules()?;
    let action = r.derived.clone().ok_or("derived action")?;
    assert!(
        mush_core::Engine::with_modules(
            World::new_minimal(),
            mush_core::Registry::new(),
            modules(&r, 0)
        )
        .is_err()
    );
    assert!(
        mush_core::Engine::with_modules(
            World::new_minimal(),
            mush_core::Registry::new(),
            vec![Box::new(
                crate::EconomyModule::new().with_derived_mind_updates(action)
            )]
        )
        .is_err()
    );
    assert!(
        mush_core::Engine::with_modules(
            World::new_minimal(),
            mush_core::Registry::new(),
            vec![
                Box::new(mush_org::OrgModule::new()),
                Box::new(r.clone()),
                Box::new(
                    crate::EconomyModule::new()
                        .with_initial_filing(policy(100), r.initial.clone())
                        .with_compatible_mind_updates(r.update.clone())
                        .with_derived_mind_updates(r.update.clone())
                ),
            ]
        )
        .is_err()
    );
    Ok(())
}

#[path = "tests/major.rs"]
mod major;

#[path = "tests/house.rs"]
mod house;

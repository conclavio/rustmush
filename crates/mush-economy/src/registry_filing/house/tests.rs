//! Actual org commands and bounded treasury funding feed the scoped E4 leg.
//! This isolates its owner images/history; the full Registry child tests the
//! coupled publication. No source, certificate or organization grant is faked.
use super::*;
use crate::registry_filing::{
    InitialFilingParticipant, InitialFilingPolicy, ReceiptAudit, receipt,
};
use crate::{CatalogRef, EconomyModule};
use mush_core::participant::{BoundParticipant, OwnerParticipant};
use mush_core::{
    ArgShape, CmdError, CommandSpec, Ctx, GOD, GameModule, ModError, ModuleBootData, ModuleCtx,
    Registrar,
};
use mush_org::{
    AccessPolicy, OrgAction, OrgModule, OrgMutationOp, OrgMutationPlan, OrgStore, PolicyEffect,
    PolicyRule, PolicySubject, ScopeSelector, SystemPrincipalId,
};
use mush_testkit::TestWorld;
use std::sync::Arc;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
const CAPS: [&str; 3] = [
    HOUSE_PUBLICATION_CAPABILITY,
    "asset.title.transfer",
    "economy.account.post",
];

/// Org has no textual policy writer. This test-only owner command uses its real
/// public submit service and audited allocator, with an explicit GOD gate.
struct PolicyOrg(OrgModule);
impl GameModule for PolicyOrg {
    fn name(&self) -> &'static str {
        "org"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        self.0.register(reg)?;
        reg.command(
            CommandSpec {
                name: "@house-policy".into(),
                shape: ArgShape::Raw,
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| {
                if ctx.actor != GOD {
                    return Err(CmdError::msg("fixture operator"));
                }
                let fields: Vec<_> = ctx.args.raw().split_whitespace().collect();
                if fields.len() != 3 {
                    return Err(CmdError::msg("fixture policy fields"));
                }
                let actor = ObjId(fields[0].parse().map_err(|_| CmdError::msg("actor"))?);
                let scope = OrgScopeId(fields[1].parse().map_err(|_| CmdError::msg("scope"))?);
                let effect = match fields[2] {
                    "allow" => PolicyEffect::Allow,
                    "deny" => PolicyEffect::Deny,
                    _ => return Err(CmdError::msg("effect")),
                };
                let store = ctx
                    .w
                    .module_state_mut::<OrgStore>()
                    .ok_or_else(|| CmdError::msg("org owner"))?;
                let org = store
                    .state()
                    .scope(scope)
                    .ok_or_else(|| CmdError::msg("scope"))?
                    .org;
                let id = store.alloc_policy();
                let rules = CAPS
                    .into_iter()
                    .map(|name| {
                        Ok(PolicyRule {
                            effect,
                            subject: PolicySubject::Principal(PrincipalId::Object(actor)),
                            capability: CapabilityId::new(name)
                                .ok_or_else(|| CmdError::msg("capability"))?,
                            scope: ScopeSelector::Exact(scope),
                            conditions: vec![],
                        })
                    })
                    .collect::<Result<Vec<_>, CmdError>>()?;
                let plan = OrgMutationPlan::of(vec![OrgMutationOp::PutPolicy {
                    policy: AccessPolicy {
                        id,
                        org,
                        owner: PrincipalId::Organization(org),
                        rules,
                        default: PolicyEffect::Deny,
                        revision: 1,
                    },
                }]);
                mush_org::submit(
                    ctx,
                    &plan,
                    OrgAction {
                        action: "house-fixture.policy".into(),
                        org: Some(org),
                        detail: serde_json::json!({"policy":id}),
                    },
                )
                .map_err(|e| CmdError::msg(e.to_string()))
            }),
        )?;
        Ok(())
    }
    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        data: ModuleBootData,
    ) -> Result<(), ModError> {
        self.0.on_world_loaded(ctx, data)
    }
}
struct Fixture {
    world: TestWorld,
    actor: ObjId,
    shell: ObjId,
    organization: OrgId,
    scope: OrgScopeId,
    other_scope: OrgScopeId,
    payer: AccountId,
    sink: AccountId,
    adapter: InitialFilingParticipant,
}
fn command(world: &mut TestWorld, line: &str) -> TestResult {
    let output = world.run(GOD, line);
    if output.iter().any(|(_, event)| matches!(event, mush_proto::OutputEvent::Sys(text) if text.spans().iter().any(|s| s.tag.as_deref()==Some("error")))) {
        return Err(format!("fixture setup refused: {line}").into());
    }
    Ok(())
}
impl Fixture {
    #[inline(never)]
    fn new() -> TestResult<Box<Self>> {
        let issuer = SystemPrincipalId::new("roll.registry").ok_or("issuer")?;
        let mut world = TestWorld::with_modules(vec![
            Box::new(PolicyOrg(OrgModule::new())),
            Box::new(EconomyModule::new().with_title_issuers(vec![issuer.clone()])),
        ]);
        let actor = world.spawn_player("Curator");
        let shell = world
            .world_mut()
            .create(ObjKind::Thing, "Reserved charter", GOD, None)?;
        for line in [
            "@org/create House",
            "@org/scope create House=publication",
            "@org/scope create House=unrelated",
            "@instrument/register cr=0",
            "@account/system faucet=cr",
            "@account/system roll.registry=cr",
        ] {
            command(&mut world, line)?;
        }
        let state = world
            .world()
            .module_state::<OrgStore>()
            .ok_or("org owner")?
            .state();
        let organization = state.orgs().next().ok_or("organization")?.id;
        let scope = state
            .scopes_in(organization)
            .find(|s| s.name.as_str() == "publication")
            .ok_or("scope")?
            .id;
        let other_scope = state
            .scopes_in(organization)
            .find(|s| s.name.as_str() == "unrelated")
            .ok_or("other scope")?
            .id;
        command(
            &mut world,
            &format!("@account/open org:House/{}=cr", scope.0),
        )?;
        let economy = world
            .world()
            .module_state::<EconomyStore>()
            .ok_or("economy owner")?;
        let payer = economy
            .snapshot()
            .accounts()
            .find(|a| a.owner == PrincipalId::Organization(organization) && a.scope == Some(scope))
            .ok_or("org wallet")?
            .id;
        let sink = economy
            .snapshot()
            .accounts()
            .find(|a| a.owner == PrincipalId::System(issuer.clone()))
            .ok_or("sink")?
            .id;
        let faucet_owner =
            PrincipalId::System(SystemPrincipalId::new("faucet").ok_or("faucet id")?);
        let faucet = economy
            .snapshot()
            .accounts()
            .find(|a| a.owner == faucet_owner)
            .ok_or("faucet")?
            .id;
        command(&mut world, &format!("@econ/mint {faucet}={payer} 1000"))?;
        let adapter = InitialFilingParticipant::new(InitialFilingPolicy::new(
            issuer,
            sink,
            Amount::from_minor(100),
            CatalogRef {
                ruleset: "roll".into(),
                code: "mind_charter".into(),
            },
        )?);
        Ok(Box::new(Self {
            world,
            actor,
            shell,
            organization,
            scope,
            other_scope,
            payer,
            sink,
            adapter,
        }))
    }
    fn state(&self) -> TestResult<EconomyStore> {
        let mut state = self
            .world
            .world()
            .module_state::<EconomyStore>()
            .ok_or("economy owner")?
            .clone();
        // Replay the complete actual setup history, not a public health setter.
        check_eq!(
            state.journal.next_seq(crate::JOURNAL_LOG),
            u64::try_from(state.transactions.len())?
        );
        let mut audit = ReceiptAudit::default();
        let mut transactions: Vec<_> = state.transactions.values().collect();
        transactions.sort_by_key(|tx| tx.journal_seq);
        for tx in transactions {
            audit.observe(tx);
        }
        audit.finish_replay()?;
        state.fold.filing_receipts = audit;
        Ok(state)
    }
    fn input(&self, actor: ObjId) -> TestResult<(RegistryInitialFiling, HouseFilingAttribution)> {
        Ok(RegistryHouseFiling {
            shell: self.shell,
            payer: self.payer,
            organization: self.organization,
            scope: self.scope,
        }
        .into_request(actor)?)
    }
    fn allow(&mut self, actor: ObjId, scope: OrgScopeId) -> TestResult {
        for capability in CAPS {
            command(
                &mut self.world,
                &format!(
                    "@org/grant #{}=House {capability} scope=#{}",
                    actor.0, scope.0
                ),
            )?;
        }
        Ok(())
    }
    fn prepare(
        &self,
        state: &EconomyStore,
        actor: ObjId,
    ) -> TestResult<(
        super::super::InitialFilingOutput,
        super::super::InitialFilingPatch,
        Vec<mush_core::authoritative::ReplayWrite>,
    )> {
        let (request, house) = self.input(actor)?;
        Ok(self.adapter.prepare_filing_images(
            state,
            self.world.world(),
            actor,
            mush_core::CommitToken::from_bytes([91; 16]),
            request,
            Some(house),
        )?)
    }
}

#[test]
fn house_requires_each_real_scoped_right_even_for_staff_and_foreign_scope() -> TestResult {
    let mut f = Fixture::new()?;
    let state = f.state()?;
    let before = state.snapshot.clone();
    check!(f.prepare(&state, GOD).is_err());
    check!(f.prepare(&state, f.actor).is_err());
    f.allow(f.actor, f.other_scope)?;
    check!(f.prepare(&state, f.actor).is_err());
    for (n, capability) in CAPS.into_iter().enumerate() {
        command(
            &mut f.world,
            &format!(
                "@org/grant #{}=House {capability} scope=#{}",
                f.actor.0, f.scope.0
            ),
        )?;
        check_eq!(f.prepare(&state, f.actor).is_ok(), n == 2);
    }
    check_eq!(state.snapshot, before);
    Ok(())
}

#[test]
fn policy_allow_is_real_authority_and_deny_revokes_before_custody_without_charge() -> TestResult {
    let mut f = Fixture::new()?;
    let state = f.state()?;
    command(
        &mut f.world,
        &format!("@house-policy {} {} allow", f.actor.0, f.scope.0),
    )?;
    let (_, patch, _) = f.prepare(&state, f.actor)?;
    command(
        &mut f.world,
        &format!("@house-policy {} {} deny", f.actor.0, f.scope.0),
    )?;
    check!(
        f.adapter
            .validate_for(
                &state,
                f.world.world(),
                f.actor,
                &patch.request,
                &patch.plan,
                patch.house.as_ref()
            )
            .is_err()
    );
    check!(f.prepare(&state, f.actor).is_err());
    check!(state.snapshot.asset(f.shell).is_none());
    check_eq!(
        state
            .snapshot
            .account(f.payer)
            .ok_or("wallet")?
            .posted_balance,
        Amount::from_minor(1000)
    );
    Ok(())
}

#[test]
fn scoped_approval_refuses_without_title_charge_or_approval_side_effect() -> TestResult {
    let mut f = Fixture::new()?;
    f.allow(f.actor, f.scope)?;
    command(
        &mut f.world,
        &format!("@account/approval {}=50 threshold:1", f.payer),
    )?;
    let state = f.state()?;
    let before = state.snapshot.clone();
    check!(f.prepare(&state, f.actor).is_err());
    check_eq!(state.snapshot, before);
    check!(state.snapshot.approvals().next().is_none());
    Ok(())
}

#[test]
fn exact_fee_limit_and_actual_scoped_payer_are_required() -> TestResult {
    let mut f = Fixture::new()?;
    for capability in [HOUSE_PUBLICATION_CAPABILITY, "asset.title.transfer"] {
        command(
            &mut f.world,
            &format!(
                "@org/grant #{}=House {capability} scope=#{}",
                f.actor.0, f.scope.0
            ),
        )?;
    }
    command(
        &mut f.world,
        &format!(
            "@org/grant #{}=House economy.account.post scope=#{} limit=99",
            f.actor.0, f.scope.0
        ),
    )?;
    let state = f.state()?;
    check!(f.prepare(&state, f.actor).is_err(), "the actual fee is 100");
    command(
        &mut f.world,
        &format!(
            "@org/grant #{}=House economy.account.post scope=#{} limit=100",
            f.actor.0, f.scope.0
        ),
    )?;
    check!(f.prepare(&state, f.actor).is_ok());
    let (mut request, house) = f.input(f.actor)?;
    request.payer = f.sink;
    check!(
        super::check(
            f.world.world(),
            &state,
            f.actor,
            &request,
            &house,
            Amount::from_minor(100)
        )
        .is_err()
    );
    let (mut request, house) = f.input(f.actor)?;
    request.scope = Some(f.other_scope);
    check!(
        super::check(
            f.world.world(),
            &state,
            f.actor,
            &request,
            &house,
            Amount::from_minor(100)
        )
        .is_err()
    );
    check!(state.snapshot.asset(f.shell).is_none());
    check_eq!(
        state
            .snapshot
            .account(f.payer)
            .ok_or("payer")?
            .posted_balance,
        Amount::from_minor(1000)
    );
    Ok(())
}

#[test]
fn actual_house_fee_title_and_history_install_as_one_owner_image() -> TestResult {
    let mut f = Fixture::new()?;
    f.allow(f.actor, f.scope)?;
    let mut state = f.state()?;
    let before = state.snapshot.clone();
    let (output, patch, writes) = f.prepare(&state, f.actor)?;
    check_eq!(state.snapshot, before);
    check_eq!(
        output.title().title_owner,
        PrincipalId::Organization(f.organization)
    );
    check_eq!(output.title().scope, Some(f.scope));
    check_eq!(output.house().ok_or("attribution")?.curator(), f.actor);
    check!(writes.iter().all(|w| f.adapter.accepts_write(w)));
    Arc::new(f.adapter.clone())
        .bind(&mut state, patch.into())?
        .apply();
    check_eq!(
        state
            .snapshot
            .account(f.payer)
            .ok_or("wallet")?
            .posted_balance,
        Amount::from_minor(900)
    );
    check_eq!(
        state.snapshot.account(f.sink).ok_or("sink")?.posted_balance,
        Amount::from_minor(100)
    );
    let tx = state
        .transaction(output.transaction())
        .ok_or("actual journal")?;
    let subject = receipt::subject_of(tx)?;
    state.filing_receipt_audit().verify(&subject)?;
    let mut replay = ReceiptAudit::default();
    replay.observe(tx);
    replay.finish_replay()?;
    replay.verify(&subject)?;
    for kind in 0..3 {
        let mut forged = tx.clone();
        match kind {
            0 => forged.asset_effects.first_mut().ok_or("title")?.post.scope = None,
            1 => {
                forged
                    .asset_effects
                    .first_mut()
                    .ok_or("title")?
                    .post
                    .title_owner = PrincipalId::Object(f.actor)
            }
            _ => {
                if let crate::EconomicCause::Ruleset { correlation, .. } = &mut forged.cause {
                    *correlation =
                        receipt::correlation(output.commit_token(), f.payer, f.sink).into();
                } else {
                    return Err("cause".into());
                }
            }
        }
        let mut audit = ReceiptAudit::default();
        audit.observe(&forged);
        check!(audit.finish_replay().is_err());
    }
    let mut checkpoint: ReceiptAudit = serde_json::from_value(serde_json::to_value(&replay)?)?;
    check!(checkpoint.require_complete().is_err());
    check!(checkpoint.checkpoint_complete());
    checkpoint.finish_replay()?;
    checkpoint.verify(&subject)?;
    Ok(())
}

//! Actual scoped Org and same-slot Economy composition. These fixtures prove
//! native fee custody and history; they do not fabricate a Registry certificate.
use super::*;
use mush_core::{ArgShape, CmdError, CommandSpec, Ctx, ModuleCtx};
use mush_org::{
    AccessPolicy, AuthRequest, CapabilityId, OrgAction, OrgId, OrgModule, OrgMutationOp,
    OrgMutationPlan, OrgScopeId, OrgStore, PolicyCondition, PolicyEffect, PolicyRule,
    PolicySubject, ScopeSelector,
};
use std::sync::Arc;

pub(super) struct PolicyOrg(pub(super) OrgModule);
impl GameModule for PolicyOrg {
    fn name(&self) -> &'static str {
        "org"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> std::result::Result<(), ModError> {
        self.0.register(reg)?;
        reg.command(
            CommandSpec {
                name: "@house-update-policy".into(),
                shape: ArgShape::Raw,
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| {
                if ctx.actor != GOD {
                    return Err(CmdError::msg("fixture operator"));
                }
                let mut fields = ctx.args.raw().split_whitespace();
                let actor = ObjId(
                    fields
                        .next()
                        .ok_or_else(|| CmdError::msg("actor"))?
                        .parse()
                        .map_err(|_| CmdError::msg("actor"))?,
                );
                let scope = OrgScopeId(
                    fields
                        .next()
                        .ok_or_else(|| CmdError::msg("scope"))?
                        .parse()
                        .map_err(|_| CmdError::msg("scope"))?,
                );
                let mode = fields.next().ok_or_else(|| CmdError::msg("mode"))?;
                if fields.next().is_some() {
                    return Err(CmdError::msg("fields"));
                }
                let store = ctx
                    .w
                    .module_state_mut::<OrgStore>()
                    .ok_or_else(|| CmdError::msg("org"))?;
                let org = store
                    .state()
                    .scope(scope)
                    .ok_or_else(|| CmdError::msg("scope"))?
                    .org;
                let prior = store
                    .state()
                    .policies_in(org)
                    .find(|p| {
                        p.rules.iter().any(|r| {
                            r.subject == PolicySubject::Principal(PrincipalId::Object(actor))
                        })
                    })
                    .cloned();
                let id = match &prior {
                    Some(p) => p.id,
                    None => store.alloc_policy(),
                };
                let effect = match mode {
                    "birth" | "allow" | "expires" | "revoke" => PolicyEffect::Allow,
                    "deny" => PolicyEffect::Deny,
                    _ => return Err(CmdError::msg("mode")),
                };
                let capabilities: &[&str] = match mode {
                    "birth" => &[
                        HOUSE_PUBLICATION_CAPABILITY,
                        "economy.account.post",
                        "asset.title.transfer",
                    ],
                    "revoke" => &[],
                    _ => &[HOUSE_PUBLICATION_CAPABILITY, "economy.account.post"],
                };
                let rules = capabilities
                    .iter()
                    .map(|name| {
                        Ok(PolicyRule {
                            effect,
                            subject: PolicySubject::Principal(PrincipalId::Object(actor)),
                            capability: CapabilityId::new(*name)
                                .ok_or_else(|| CmdError::msg("capability"))?,
                            scope: ScopeSelector::Exact(scope),
                            conditions: if mode == "expires" {
                                vec![PolicyCondition::TimeWindow {
                                    from: None,
                                    until: Some(200),
                                }]
                            } else {
                                vec![]
                            },
                        })
                    })
                    .collect::<std::result::Result<Vec<_>, CmdError>>()?;
                let plan = OrgMutationPlan::of(vec![OrgMutationOp::PutPolicy {
                    policy: AccessPolicy {
                        id,
                        org,
                        owner: PrincipalId::Organization(org),
                        rules,
                        default: PolicyEffect::Deny,
                        revision: prior.map_or(1, |p| p.revision + 1),
                    },
                }]);
                mush_org::submit(
                    ctx,
                    &plan,
                    OrgAction {
                        action: "house-update-fixture.policy".into(),
                        org: Some(org),
                        detail: json!({"policy":id}),
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
    ) -> std::result::Result<(), ModError> {
        self.0.on_world_loaded(ctx, data)
    }
}
fn house_rules() -> Result<Rules> {
    let mut r = derived_rules()?;
    for id in [HOUSE_FILING_ACTION, HOUSE_ACTION_ID] {
        let d = descriptor("action", id)?;
        let action = DefinitionRegistryBuilder::new().register(OwnerId::new("roll")?, d.clone())?;
        r.declarations.push(d);
        if id == HOUSE_FILING_ACTION {
            r.house_initial = Some(action);
        } else {
            r.house_update = Some(action);
        }
    }
    Ok(r)
}
fn command(f: &mut Fixture, line: &str) -> Result {
    let events = f.world.run(GOD, line);
    if events.iter().any(|(_,e)|matches!(e,mush_proto::OutputEvent::Sys(t) if t.spans().iter().any(|s|s.tag.as_deref()==Some("error")))) {
        return Err("actual House fixture command refused".into());
    }
    fold_module_ops_into(&mut f.boot, f.world.take_module_ops());
    let _ = f.world.world_mut().take_dirty();
    Ok(())
}
fn policy_command(f: &mut Fixture, actor: ObjId, scope: OrgScopeId, mode: &str) -> Result {
    command(
        f,
        &format!("@house-update-policy {} {} {mode}", actor.0, scope.0),
    )
}
fn parameters(f: &Fixture, org: OrgId, scope: OrgScopeId) -> Value {
    json!({"charter":f.shell,"payer":1,"title_revision":f.state().snapshot.asset(f.shell).map_or(0,|a|a.revision),"organization":org,"scope":scope})
}
#[inline(never)]
fn house_fixture() -> Result<(Box<Fixture>, ObjId, OrgId, OrgScopeId)> {
    let mut f = Fixture::empty_rules(house_rules()?)?;
    f.world.set_now(100);
    for line in [
        "@org/create House",
        "@org/scope create House=publication",
        "@instrument/register cr=0",
        "@account/system faucet=cr",
        "@account/open org:House/0=cr",
        "@account/system roll.registry=cr",
        "@econ/mint 0=1 10000",
    ] {
        command(&mut f, line)?;
    }
    let org = f
        .world
        .world()
        .module_state::<OrgStore>()
        .ok_or("Org")?
        .state()
        .orgs()
        .next()
        .ok_or("actual org")?
        .id;
    let scope = f
        .world
        .world()
        .module_state::<OrgStore>()
        .ok_or("Org")?
        .state()
        .scopes_in(org)
        .next()
        .ok_or("actual scope")?
        .id;
    assert_eq!(scope, OrgScopeId(0), "actual first scope allocator");
    let first = f.actor;
    policy_command(&mut f, first, scope, "birth")?;
    let initial = f.rules.house_initial.clone().ok_or("House initial")?;
    let input = parameters(&f, org, scope);
    let p = f.prepare_parameters(&initial, first, 1, input)?;
    f.publish(initial, 1, p)?;
    let next = f.world.spawn_player("Replacement curator");
    policy_command(&mut f, next, scope, "expires")?;
    Ok((f, next, org, scope))
}
fn update(
    f: &mut Fixture,
    actor: ObjId,
    org: OrgId,
    scope: OrgScopeId,
    n: u8,
) -> Result<PreparedAction> {
    let action = f.rules.house_update.clone().ok_or("House update")?;
    f.prepare_parameters(&action, actor, n, parameters(f, org, scope))
}
fn receipt_for(f: &Fixture, n: u8) -> Result<UpdateAuditSubject> {
    f.state()
        .update_receipt_audit()
        .receipts()?
        .find(|r| r.commit == token(n))
        .cloned()
        .ok_or_else(|| "actual receipt".into())
}

#[inline(never)]
fn registered(
    modules: Vec<Box<dyn GameModule>>,
) -> std::result::Result<Box<mush_core::Engine>, ModError> {
    mush_core::Engine::with_modules(World::new_minimal(), mush_core::Registry::new(), modules)
        .map(Box::new)
}
#[inline(never)]
fn cold(f: &Fixture, rules: &Rules) -> Result<Box<mush_core::Engine>> {
    let (_, registry) = mush_testkit::boot_world();
    let mut engine =
        mush_core::Engine::with_modules(f.world.world().clone(), registry, modules(rules, 100))
            .map(Box::new)?;
    let mut boot = f.boot.clone();
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
    engine.hydrate_action_state(typed)?;
    engine.modules_loaded(
        boot,
        &mut mush_core::Outbox::new(),
        &mut mush_core::engine::Effects::new(),
    )?;
    Ok(engine)
}

#[test]
fn house_update_charges_real_scoped_treasury_for_current_curator_without_title_effect() -> Result {
    let (mut f, curator, org, scope) = house_fixture()?;
    let before = f.state().snapshot.clone();
    let title = before.asset(f.shell).ok_or("House title")?.clone();
    let birth = serde_json::to_value(f.state().filing_receipt_audit())?;
    assert_ne!(curator, f.actor);
    assert!(
        !mush_org::authorize_in_world(
            f.world.world(),
            &AuthRequest {
                actor: curator,
                capability: CapabilityId::new("asset.title.transfer").ok_or("cap")?,
                scope,
                now: 100,
                amount: None,
            }
        )
        .is_allow(),
        "fee-only update needs no title transfer right"
    );
    assert!(
        update(&mut f, GOD, org, scope, 2).is_err(),
        "no staff fallback"
    );
    let action = f.rules.house_update.clone().ok_or("action")?;
    let p = update(&mut f, curator, org, scope, 2)?;
    assert_eq!(f.state().snapshot, before);
    let writes = f.stage(action, 2, p)?;
    assert_eq!(f.state().snapshot, before);
    let actual = prepared_receipt(
        &writes,
        &f.rules,
        &f.rules.house_update.clone().ok_or("action")?,
        token(2),
        curator,
    )?;
    f.take_install()?;
    actual.verify(f.state())?;
    let subject = receipt_for(&f, 2)?;
    assert_eq!(subject.kind, Some(UpdateKind::HouseCompatibleV1));
    assert_eq!(
        subject.house,
        Some(HouseFilingAttribution::new(curator, org, scope)?)
    );
    assert_eq!(f.state().snapshot.asset(f.shell), Some(&title));
    assert_eq!(
        serde_json::to_value(f.state().filing_receipt_audit())?,
        birth
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
    assert!(!writes.iter().any(
        |w| matches!(w,ReplayWrite::ModuleJsonSet{key,..} if key.as_str().starts_with("asset:"))
    ));
    let tx = f
        .state()
        .transaction(subject.transaction)
        .ok_or("native transaction")?;
    assert_eq!(receipt::subject_of(tx)?, subject);
    assert_eq!(tx.idempotency.namespace.as_str(), receipt::HOUSE_NAMESPACE);
    assert_eq!(
        crate::contract::managed::require_legacy_transaction(tx, &f.state().snapshot),
        Err(crate::EconomyError::ManagedContract)
    );
    Ok(())
}

#[test]
fn house_update_repeats_scoped_authorization_readiness_and_money_at_all_custody_boundaries()
-> Result {
    for phase in 0..3 {
        for drift in [
            "deny", "revoke", "expiry", "org", "economy", "title", "payer", "going",
        ] {
            let (mut f, curator, org, scope) = house_fixture()?;
            let action = f.rules.house_update.clone().ok_or("action")?;
            let mut prepared = if phase > 0 {
                Some(update(&mut f, curator, org, scope, 2)?)
            } else {
                None
            };
            if phase == 2 {
                f.stage(action.clone(), 2, prepared.take().ok_or("prepared")?)?;
            }
            let shell = f.shell;
            match drift {
                "deny" | "revoke" => policy_command(&mut f, curator, scope, drift)?,
                "expiry" => f.world.set_now(200),
                "org" | "economy" => {
                    f.world.engine_mut().registry.freeze_mechanics_owner(
                        &OwnerId::new(drift)?,
                        mush_core::mechanics::IncidentId(19),
                        mush_core::mechanics::RecoveryReason::IntegrityViolation {
                            code: CanonicalId::new("house_update_fixture")?,
                        },
                    )?;
                }
                "title" => {
                    f.state_mut()
                        .snapshot
                        .asset_mut(shell)
                        .ok_or("title")?
                        .scope = Some(OrgScopeId(99))
                }
                "payer" => {
                    f.state_mut()
                        .snapshot
                        .account_mut(AccountId(1))
                        .ok_or("payer")?
                        .status = AccountStatus::Frozen
                }
                "going" => {
                    f.world
                        .world_mut()
                        .get_mut(curator)
                        .ok_or("curator")?
                        .flags
                        .insert(mush_core::Flags::GOING);
                    let _ = f.world.world_mut().take_dirty();
                }
                _ => return Err("drift".into()),
            }
            let before = f.state().snapshot.clone();
            let next = f.state().next_transaction;
            match phase {
                0 => assert!(
                    update(&mut f, curator, org, scope, 2).is_err(),
                    "prepare {drift}"
                ),
                1 => assert!(
                    f.stage(action, 2, prepared.take().ok_or("prepared")?)
                        .is_err(),
                    "stage {drift}"
                ),
                _ => assert!(
                    f.world.engine_mut().take_staged_action().is_err(),
                    "take {drift}"
                ),
            }
            assert_eq!(f.state().snapshot, before);
            assert_eq!(f.state().next_transaction, next);
            assert_eq!(f.state().update_receipt_audit().receipts()?.len(), 0);
        }
    }
    Ok(())
}

#[test]
fn house_update_cold_inverse_and_closed_discriminator_preserve_old_omission() -> Result {
    let (mut f, curator, org, scope) = house_fixture()?;
    let action = f.rules.house_update.clone().ok_or("action")?;
    let p = update(&mut f, curator, org, scope, 2)?;
    f.publish(action, 2, p)?;
    let subject = receipt_for(&f, 2)?;
    let restored = cold(&f, &house_rules()?)?;
    restored
        .world
        .module_state::<EconomyStore>()
        .ok_or("actual cold economy")?
        .update_receipt_audit()
        .verify(&subject)?;
    assert!(cold(&f, &derived_rules()?).is_err());
    let tx = f
        .state()
        .transaction(subject.transaction)
        .ok_or("transaction")?;
    let audit = f.state().update_receipt_audit();
    audit.require_all_protocols(true, true, false, true)?;
    assert!(audit.require_protocols(true, true, true).is_err());
    let rows = vec![crate::checkpoint::IdemRow {
        ns: receipt::HOUSE_NAMESPACE.into(),
        key: subject.commit.to_hex().into(),
        plan_hash: subject
            .plan_hash
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
        tx: subject.transaction,
        posted_at: subject.posted_at_utc,
        seq: subject.journal_seq,
    }];
    assert!(audit.checkpoint_complete(&rows));
    assert!(!audit.checkpoint_complete(&[]));
    assert!(!UpdateReceiptAudit::default().checkpoint_complete(&rows));
    let mut wrong = rows.clone();
    wrong.first_mut().ok_or("row")?.ns = receipt::NAMESPACE.into();
    assert!(!audit.checkpoint_complete(&wrong));
    let mut crossed = tx.clone();
    crossed.idempotency.namespace = receipt::NAMESPACE.into();
    let mut invalid = UpdateReceiptAudit::default();
    invalid.observe(&crossed);
    assert!(invalid.finish_replay().is_err());
    for change in 0..4 {
        let mut value = serde_json::to_value(&subject)?;
        match change {
            0 => {
                value.as_object_mut().ok_or("object")?.remove("house");
            }
            1 => value["house"] = Value::Null,
            2 => value["kind"] = json!("dependent_v1"),
            _ => value["house"]["curator"] = json!(GOD),
        };
        let parsed = serde_json::from_value::<UpdateAuditSubject>(value);
        assert!(parsed.is_err() || parsed.is_ok_and(|r| r.validate().is_err()));
    }
    let mut personal = Fixture::new_derived()?;
    for (a, n) in [
        (personal.rules.update.clone(), 2),
        (personal.rules.derived.clone().ok_or("derived")?, 3),
    ] {
        let actor = personal.actor;
        let p = personal.prepare(&a, actor, n)?;
        personal.publish(a, n, p)?;
        assert!(
            serde_json::to_value(receipt_for(&personal, n)?)?
                .get("house")
                .is_none()
        );
    }
    // Full-width decimal identities fit the House-only cause bound; older causes
    // still use their unchanged six-field correlation and 128-byte admission.
    let h = HouseFilingAttribution::new(ObjId(u32::MAX), OrgId(u64::MAX), OrgScopeId(u64::MAX))?;
    let request = RegistryCompatibleUpdate {
        charter: ObjId(u32::MAX),
        title_revision: u64::MAX,
        payer: AccountId(u64::MAX),
    };
    let cause = receipt::correlation_for(
        token(255),
        h.curator(),
        &request,
        AccountId(u64::MAX - 1),
        Some(&h),
    );
    assert!(cause.len() <= 192);
    Ok(())
}

#[test]
fn house_update_requires_exact_registration_and_initial_policy_in_either_owner_order() -> Result {
    let r = house_rules()?;
    for reverse in [false, true] {
        let mut m = modules(&r, 100);
        if reverse {
            m.reverse();
        }
        let _engine = registered(m)?;
    }
    assert!(registered(modules(&r, 0)).is_err());
    let action = r.house_update.clone().ok_or("action")?;
    assert!(
        registered(vec![Box::new(
            crate::EconomyModule::new().with_house_mind_updates(action)
        )])
        .is_err()
    );
    let mut wrong = modules(&r, 100);
    wrong.pop();
    wrong.push(Box::new(
        crate::EconomyModule::new()
            .with_initial_filing(policy(100), r.initial.clone())
            .with_compatible_mind_updates(r.update.clone())
            .with_derived_mind_updates(r.derived.clone().ok_or("derived")?)
            .with_house_initial_filing(r.house_initial.clone().ok_or("initial")?)
            .with_house_mind_updates(r.update.clone()),
    ));
    assert!(registered(wrong).is_err());
    Ok(())
}

//! Pure codec bounds and the actual Org/Engine authority seam. No certificate,
//! commerce readiness, recall publication, or durable business receipt is faked.
use super::*;
use mush_core::{
    ArgShape, CmdError, CommandSpec, Ctx, Engine, GOD, GameModule, Inbound, ModuleBootData,
    ModuleCtx, ModuleWrite, ObjKind, Outbox, Registrar, Registry, World,
};
use mush_org::{
    AccessPolicy, OrgAction, OrgId, OrgModule, OrgMutationOp, OrgMutationPlan, OrgScopeId,
    OrgStore, PolicyEffect, PolicyRule, PolicySubject, ScopeSelector, SystemPrincipalId,
};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex},
};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
type Pending = Arc<Mutex<Option<OrgMutationPlan>>>;
struct TestOrg {
    inner: OrgModule,
    pending: Pending,
}
impl GameModule for TestOrg {
    fn name(&self) -> &'static str {
        "org"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> std::result::Result<(), ModError> {
        self.inner.register(reg)?;
        let pending = self.pending.clone();
        reg.command(
            CommandSpec {
                name: "@recall-org-fixture".into(),
                shape: ArgShape::Raw,
                ..Default::default()
            },
            Arc::new(move |ctx: &mut Ctx<'_>| {
                if ctx.actor != GOD {
                    return Err(CmdError::msg("fixture operator"));
                }
                let plan = pending
                    .lock()
                    .map_err(|_| CmdError::msg("fixture lock"))?
                    .take()
                    .ok_or_else(|| CmdError::msg("fixture plan"))?;
                mush_org::submit(
                    ctx,
                    &plan,
                    OrgAction {
                        action: "recall-fixture.policy".into(),
                        org: None,
                        detail: json!({"fixture":true}),
                    },
                )
                .map_err(|_| CmdError::msg("actual Org mutation refused"))
            }),
        )?;
        Ok(())
    }
    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        data: ModuleBootData,
    ) -> std::result::Result<(), ModError> {
        self.inner.on_world_loaded(ctx, data)
    }
}
/// Registers real definitions and a real Roll-owned slot only. It never opens
/// commerce; these tests exercise authorization independently of a filing.
struct AuthorityRoll {
    policy: RegistryRecallPolicy,
    ready: Readiness,
}
impl GameModule for AuthorityRoll {
    fn name(&self) -> &'static str {
        crate::NS
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> std::result::Result<(), ModError> {
        reg.state(crate::RollStore::default());
        let mut definitions = self.policy.definitions()?.to_vec();
        definitions.push(super::super::rating::registration::commercial_roots_descriptor()?);
        for kind in [
            "action",
            "outcome",
            "owner_participant",
            "registry_authority",
        ] {
            let expected = definitions
                .iter()
                .filter(|definition| definition.kind().as_str() == kind)
                .cloned()
                .collect::<Vec<_>>();
            reg.definition_validator(
                mush_core::mechanics::DefinitionKind::new(kind)
                    .map_err(|error| ModError::msg(error.to_string()))?,
                move |ctx: &mush_core::mechanics::DefinitionValidationContext<'_>| {
                    super::super::module::validate_definitions(crate::NS, &expected, ctx)
                },
            )?;
        }
        for definition in definitions {
            reg.definition(definition)?;
        }
        Ok(())
    }
    fn finalize_owner_composition(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
    ) -> std::result::Result<(), ModError> {
        self.ready.capture(ctx)
    }
}
fn command(e: &mut Engine, line: &str) -> Result {
    let mut out = Outbox::new();
    e.step(
        Inbound::Cmd {
            conn: 1,
            actor: GOD,
            line: line.into(),
        },
        &mut out,
        &mut mush_core::Effects::new(),
    );
    if out.events().iter().any(|(_,e)|matches!(e,mush_proto::OutputEvent::Sys(text) if text.spans().iter().any(|s|s.tag.as_deref()==Some("error")))) {return Err("actual Org fixture command refused".into());}
    Ok(())
}
fn boot(e: &mut Engine, data: HashMap<smol_str::SmolStr, ModuleBootData>) -> Result {
    e.modules_loaded(data, &mut Outbox::new(), &mut mush_core::Effects::new())?;
    Ok(())
}
struct Rig {
    engine: Engine,
    policy: RegistryRecallPolicy,
    ready: Readiness,
    pending: Pending,
    actor: ObjId,
    other: ObjId,
}
impl Rig {
    fn new() -> Result<Self> {
        let pending = Pending::default();
        let mut seed = Engine::with_modules(
            World::new_minimal(),
            Registry::new(),
            vec![Box::new(TestOrg {
                inner: OrgModule::new(),
                pending: pending.clone(),
            })],
        )?;
        seed.world.set_now(100);
        let actor = seed
            .world
            .create(ObjKind::Player, "Recall operator", GOD, None)?;
        let other = seed
            .world
            .create(ObjKind::Player, "Other operator", GOD, None)?;
        boot(&mut seed, HashMap::new())?;
        command(&mut seed, "@org/create Registry")?;
        command(&mut seed, "@org/scope create Registry=recall")?;
        let state = seed.world.module_state::<OrgStore>().ok_or("Org")?.state();
        let org = state.orgs().next().ok_or("actual org")?.id;
        let scope = state
            .scopes_in(org)
            .find(|s| s.name.as_str() == "recall")
            .ok_or("actual scope")?
            .id;
        command(
            &mut seed,
            &format!(
                "@org/grant #{}=Registry {} scope={}",
                actor.0, CAPABILITY, scope.0
            ),
        )?;
        let policy = RegistryRecallPolicy::new(
            SystemPrincipalId::new("roll.registry").ok_or("issuer")?,
            org,
            scope,
            vec![actor],
        )?;
        let ready = Readiness::default();
        let mut docs = BTreeMap::new();
        for op in seed.take_module_ops() {
            if op.ns.as_str() == "org" {
                match op.op {
                    ModuleWrite::JsonSet { key, value } => {
                        docs.insert(key, value);
                    }
                    ModuleWrite::Del { key } => {
                        docs.remove(&key);
                    }
                    ModuleWrite::StreamAppend { .. } => {}
                }
            }
        }
        let mut world = World::from_objects(seed.world.iter().cloned(), seed.world.next_id())?;
        world.set_now(seed.world.now());
        let mut engine = Engine::with_modules(
            world,
            Registry::new(),
            vec![
                Box::new(TestOrg {
                    inner: OrgModule::new(),
                    pending: pending.clone(),
                }),
                Box::new(AuthorityRoll {
                    policy: policy.clone(),
                    ready: ready.clone(),
                }),
            ],
        )?;
        boot(
            &mut engine,
            HashMap::from([(
                "org".into(),
                ModuleBootData {
                    kv: docs.into_iter().collect(),
                    ..Default::default()
                },
            )]),
        )?;
        Ok(Self {
            engine,
            policy,
            ready,
            pending,
            actor,
            other,
        })
    }
    fn observe(
        &self,
    ) -> std::result::Result<(authority::Authorization, mush_core::ClockStamp), PublicationError>
    {
        authority::observe(&self.engine.world, &self.ready, &self.policy, self.actor)
    }
    fn mutate(&mut self, op: OrgMutationOp) -> Result {
        *self.pending.lock().map_err(|_| "fixture lock")? = Some(OrgMutationPlan::of(vec![op]));
        command(&mut self.engine, "@recall-org-fixture")
    }
    fn grant(&self) -> Result<mush_org::CapabilityGrant> {
        Ok(self
            .engine
            .world
            .module_state::<OrgStore>()
            .ok_or("Org")?
            .state()
            .grants_in(self.policy.organization)
            .next()
            .ok_or("actual grant")?
            .clone())
    }
}
#[test]
fn registry_recall_actual_authorizer_has_no_staff_or_unconfigured_operator_fallback() -> Result {
    let r = Rig::new()?;
    assert!(r.observe().is_ok());
    for actor in [GOD, r.other] {
        assert!(authority::observe(&r.engine.world, &r.ready, &r.policy, actor).is_err());
    }
    assert!(
        authority::observe(&r.engine.world, &Readiness::default(), &r.policy, r.actor).is_err()
    );
    let mut altered = r.policy.clone();
    altered.scope = OrgScopeId(u64::MAX);
    assert!(authority::observe(&r.engine.world, &r.ready, &altered, r.actor).is_err());
    assert!(
        r.engine
            .world
            .module_state::<crate::RollStore>()
            .ok_or("Roll")?
            .commerce_ready()
            .is_err(),
        "authority fixture never manufactures commerce health"
    );
    Ok(())
}
#[test]
fn registry_recall_fresh_org_deny_replacement_revocation_and_expiry_are_decisive() -> Result {
    for mode in ["deny", "replaced", "revoked", "expired"] {
        let mut r = Rig::new()?;
        let mut grant = r.grant()?;
        if mode == "expired" {
            grant.valid_until = Some(105);
            r.mutate(OrgMutationOp::PutGrant {
                grant: grant.clone(),
            })?;
        }
        let (before, stamp) = r.observe()?;
        match mode {
            "deny" => {
                // A real owner mutation, not a forged witness or Boolean gate.
                let id = r
                    .engine
                    .world
                    .module_state::<OrgStore>()
                    .ok_or("Org")?
                    .state()
                    .policies_in(r.policy.organization)
                    .map(|p| p.id.0)
                    .max()
                    .map_or(0, |n| n + 1);
                r.mutate(OrgMutationOp::PutPolicy {
                    policy: AccessPolicy {
                        id: mush_org::PolicyId(id),
                        org: r.policy.organization,
                        owner: mush_org::PrincipalId::Organization(r.policy.organization),
                        rules: vec![PolicyRule {
                            effect: PolicyEffect::Deny,
                            subject: PolicySubject::Principal(mush_org::PrincipalId::Object(
                                r.actor,
                            )),
                            capability: mush_org::CapabilityId::new(CAPABILITY)
                                .ok_or("capability")?,
                            scope: ScopeSelector::Exact(r.policy.scope),
                            conditions: vec![],
                        }],
                        default: PolicyEffect::Deny,
                        revision: 1,
                    },
                })?;
            }
            "replaced" => {
                grant.grantee = r.other;
                r.mutate(OrgMutationOp::PutGrant { grant })?;
            }
            "revoked" => {
                r.mutate(OrgMutationOp::RevokeGrant { grant: grant.id })?;
            }
            "expired" => r.engine.world.set_now(105),
            _ => return Err("mode".into()),
        }
        assert!(r.observe().is_err(), "live refusal {mode}");
        assert!(
            authority::recheck(
                &r.engine.world,
                &r.ready,
                &r.policy,
                r.actor,
                &before,
                &stamp
            )
            .is_err(),
            "retained witness cannot override {mode}"
        );
    }
    Ok(())
}
#[test]
fn registry_recall_real_policy_allow_remains_subject_to_deny_precedence() -> Result {
    let mut r = Rig::new()?;
    let grant = r.grant()?;
    r.mutate(OrgMutationOp::RevokeGrant { grant: grant.id })?;
    assert!(r.observe().is_err());
    let rule = |effect| PolicyRule {
        effect,
        subject: PolicySubject::Principal(mush_org::PrincipalId::Object(r.actor)),
        capability: mush_org::CapabilityId::new(CAPABILITY).expect("fixed capability"),
        scope: ScopeSelector::Exact(r.policy.scope),
        conditions: vec![],
    };
    let allow = rule(PolicyEffect::Allow);
    let deny = rule(PolicyEffect::Deny);
    let mut policy = AccessPolicy {
        id: mush_org::PolicyId(0),
        org: r.policy.organization,
        owner: mush_org::PrincipalId::Organization(r.policy.organization),
        rules: vec![allow],
        default: PolicyEffect::Deny,
        revision: 1,
    };
    r.mutate(OrgMutationOp::PutPolicy {
        policy: policy.clone(),
    })?;
    let (accepted, stamp) = r.observe()?;
    policy.rules.push(deny);
    policy.revision = 2;
    r.mutate(OrgMutationOp::PutPolicy { policy })?;
    assert!(r.observe().is_err());
    assert!(
        authority::recheck(
            &r.engine.world,
            &r.ready,
            &r.policy,
            r.actor,
            &accepted,
            &stamp
        )
        .is_err()
    );
    Ok(())
}
#[test]
fn registry_recall_actual_owner_freeze_actor_loss_and_clock_drift_refuse_retained_observation()
-> Result {
    for mode in ["org", "roll", "going", "utc", "generation"] {
        let mut r = Rig::new()?;
        let (before, stamp) = r.observe()?;
        match mode {
            "org" | "roll" => {
                r.engine.registry.freeze_mechanics_owner(
                    &OwnerId::new(mode)?,
                    mush_core::mechanics::IncidentId(91),
                    mush_core::mechanics::RecoveryReason::IntegrityViolation {
                        code: mush_core::mechanics::CanonicalId::new("recall_fixture")?,
                    },
                )?;
            }
            "going" => r
                .engine
                .world
                .get_mut(r.actor)
                .ok_or("actor")?
                .flags
                .insert(mush_core::Flags::GOING),
            "utc" => r.engine.world.set_now(99),
            "generation" => {
                let epoch = r.engine.world.next_world_timeline_epoch(2)?;
                r.engine.world.append_world_timeline_epoch(epoch)?;
            }
            _ => return Err("mode".into()),
        }
        assert!(
            authority::recheck(
                &r.engine.world,
                &r.ready,
                &r.policy,
                r.actor,
                &before,
                &stamp
            )
            .is_err(),
            "retained observation {mode}"
        );
    }
    Ok(())
}
fn operation() -> Result<records::RecallOperation> {
    let digest = mush_core::DefinitionDigest::from_bytes([255; 32]);
    let policy = RegistryRecallPolicy::new(
        SystemPrincipalId::new("r".repeat(64)).ok_or("issuer")?,
        OrgId(u64::MAX),
        OrgScopeId(u64::MAX),
        (0..32).map(|n| ObjId(u32::MAX - 31 + n)).collect(),
    )?;
    let revision = json!({"format":1,"revision":{"key":{"character":u32::MAX,"mind":u64::MAX,"number":u64::MAX},"digest":digest},"source":digest});
    let sources: Vec<_> = (0..64)
        .map(|n| authority::Source {
            tag: "delegation".into(),
            revision: u64::MAX - 63 + n,
        })
        .collect();
    Ok(serde_json::from_value(
        json!({"format":1,"operation":CommitToken::from_bytes([255;16]),"action":policy.action()?,"authority":policy,"actor":u32::MAX,
        "authorization":{"at":i64::MIN,"organization_revision":u64::MAX,"scope_revision":u64::MAX,"sources":sources},"filed":u64::MAX.to_string(),"revision":revision,
        "rating":{"schema":1,"job":CommitToken::from_bytes([254;16]),"id":digest,"receipt_digest":digest},"initial_operation":CommitToken::from_bytes([253;16]),"initial_issuer":"r".repeat(64),"withdrawn_by":CommitToken::from_bytes([252;16]),"reason":"registry_review","at_ic":{"clock":{"kind":"world_ic","generation":u64::MAX},"at":i64::MAX}}),
    )?)
}
#[test]
fn registry_recall_closed_full_envelope_maximum_fixed_widths_and_digest_are_bounded() -> Result {
    let op = operation()?;
    op.validate()?;
    let encoded = serde_json::to_value(&op)?;
    let envelope = json!({"family":{"owner":"roll","id":records::RecallOperation::FAMILY},"schema":{"id":format!("{}.value.v1",records::RecallOperation::FAMILY),"version":1},"key":op.key(),"value":encoded});
    assert!(serde_json::to_vec(&envelope)?.len() <= records::RecallOperation::MAX_BYTES);
    assert_eq!(
        serde_json::from_value::<records::RecallOperation>(encoded.clone())?,
        op
    );
    for field in [
        "authority",
        "actor",
        "authorization",
        "filed",
        "rating",
        "initial_issuer",
        "withdrawn_by",
        "at_ic",
    ] {
        let mut bad = encoded.clone();
        bad.as_object_mut().ok_or("record")?.remove(field);
        assert!(
            serde_json::from_value::<records::RecallOperation>(bad).is_err(),
            "required {field}"
        );
    }
    for pointer in [
        "",
        "/authority",
        "/authorization",
        "/rating",
        "/revision",
        "/at_ic",
    ] {
        let mut bad = encoded.clone();
        bad.pointer_mut(pointer)
            .and_then(serde_json::Value::as_object_mut)
            .ok_or("nested")?
            .insert("authority_override".into(), json!(true));
        assert!(serde_json::from_value::<records::RecallOperation>(bad).is_err());
    }
    let mut bad = encoded.clone();
    bad.get_mut("authorization")
        .and_then(|v| v.get_mut("sources"))
        .and_then(serde_json::Value::as_array_mut)
        .ok_or("sources")?
        .push(json!({"tag":"grant","revision":1}));
    assert!(serde_json::from_value::<records::RecallOperation>(bad).is_err());
    // SystemPrincipalId has no intrinsic text length limit. The complete
    // protocol envelope, including both issuer occurrences, is authoritative.
    let mut oversized = op.clone();
    let huge = SystemPrincipalId::new("r".repeat(records::RecallOperation::MAX_BYTES))
        .ok_or("long issuer")?;
    oversized.authority.issuer = huge.clone();
    oversized.initial_issuer = huge;
    oversized.action = oversized.authority.action()?;
    assert!(matches!(
        oversized.validate(),
        Err(PublicationError::Capacity)
    ));
    let mut changed = op.clone();
    changed.authorization.at += 1;
    assert_ne!(changed.digest()?, op.digest()?);
    let wire = serde_json::to_string(&op)?;
    let duplicate = wire.replacen("\"format\":1", "\"format\":1,\"format\":1", 1);
    assert!(serde_json::from_str::<records::RecallOperation>(&duplicate).is_err());
    Ok(())
}
#[test]
fn registry_recall_policy_is_closed_bounded_and_does_not_invent_identifier_minima() -> Result {
    let issuer = SystemPrincipalId::new("roll.registry").ok_or("issuer")?;
    assert!(
        RegistryRecallPolicy::new(issuer.clone(), OrgId(0), OrgScopeId(0), vec![ObjId(0)]).is_ok()
    );
    for operators in [
        vec![],
        vec![ObjId(2), ObjId(2)],
        vec![ObjId(3), ObjId(2)],
        (0..33).map(ObjId).collect(),
    ] {
        assert!(
            RegistryRecallPolicy::new(issuer.clone(), OrgId(1), OrgScopeId(2), operators).is_err()
        );
    }
    let p = RegistryRecallPolicy::new(issuer, OrgId(1), OrgScopeId(2), vec![GOD])?;
    let mut raw = serde_json::to_value(p)?;
    raw.as_object_mut()
        .ok_or("policy")?
        .insert("god_override".into(), json!(true));
    assert!(serde_json::from_value::<RegistryRecallPolicy>(raw).is_err());
    Ok(())
}

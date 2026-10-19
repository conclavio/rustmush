//! Real Org commands, scoped E4 vaults, Inventory and Industry publication.
//! No certification row, quota, grant or readiness bit is seeded by this rig.
use super::*;
use super::{
    participant::CoreProductionState,
    provider::{Runtime, TestHandles},
    records::*,
};
use crate::IndustryModule;
use mush_core::action::{
    ActionInvocationDraft, ActionOfferId, PreparedAuthoritativeAction, PreparedCausalRef,
};
use mush_core::authoritative::{AuthoritativeBundle, AuthoritativeBundleDraft, ReplayWrite};
use mush_core::composed::PreparedComposedTxn;
use mush_core::mechanics::{DefinitionRegistryBuilder, OwnerId};
use mush_core::{
    ActionInvocation, ArgShape, CanonicalHash, CmdError, CommandSpec, CommitToken, Ctx, Engine,
    GOD, GameModule, IdempotencyKey, IdempotencyScope, ModuleBootData, ModuleCtx, ModuleOp,
    ModuleWrite, ObjId, ObjKind, Outbox, PerceivedRef, Registrar, RequestUid, ScopedIdempotencyKey,
};
use mush_economy::{EconomyModule, EconomyStore};
use mush_inventory::{Grade, InventoryHandleCell, InventoryModule, Lineage};
use mush_org::{
    AccessPolicy, CapabilityId, OrgAction, OrgModule, OrgMutationOp, OrgMutationPlan, OrgScopeId,
    OrgStore, PolicyEffect, PolicyRule, PolicySubject, PrincipalId, ScopeSelector,
};
use mush_testkit::{TestWorld, fold_module_ops, fold_module_ops_into};
use serde_json::json;
use smol_str::SmolStr;
use std::{collections::HashMap, sync::Arc};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
type Prepared = (PreparedComposedTxn, PreparedAuthoritativeAction);
type Boot = HashMap<SmolStr, ModuleBootData>;
const CAPS: [&str; 3] = [IMPORT_CAPABILITY, "vault.deposit", "vault.withdraw"];
struct PolicyOrg(OrgModule);
impl GameModule for PolicyOrg {
    fn name(&self) -> &'static str {
        "org"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> std::result::Result<(), ModError> {
        self.0.register(reg)?;
        reg.command(
            CommandSpec {
                name: "@origin-policy".into(),
                shape: ArgShape::Raw,
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| {
                if ctx.actor != GOD {
                    return Err(CmdError::msg("fixture operator"));
                }
                let fields: Vec<_> = ctx.args.raw().split_whitespace().collect();
                let [actor, scope, effect] = fields.as_slice() else {
                    return Err(CmdError::msg("fixture policy"));
                };
                let actor = ObjId(actor.parse().map_err(|_| CmdError::msg("actor"))?);
                let scope = OrgScopeId(scope.parse().map_err(|_| CmdError::msg("scope"))?);
                let effect = match *effect {
                    "allow" => PolicyEffect::Allow,
                    "deny" => PolicyEffect::Deny,
                    _ => return Err(CmdError::msg("effect")),
                };
                let store = ctx
                    .w
                    .module_state_mut::<OrgStore>()
                    .ok_or_else(|| CmdError::msg("org"))?;
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
                    .collect::<std::result::Result<Vec<_>, CmdError>>()?;
                mush_org::submit(
                    ctx,
                    &OrgMutationPlan::of(vec![OrgMutationOp::PutPolicy {
                        policy: AccessPolicy {
                            id,
                            org,
                            owner: PrincipalId::Organization(org),
                            rules,
                            default: PolicyEffect::Deny,
                            revision: 1,
                        },
                    }]),
                    OrgAction {
                        action: "origin-fixture.policy".into(),
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
struct Content([Grade; 3]);
impl GameModule for Content {
    fn name(&self) -> &'static str {
        "tacsim"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> std::result::Result<(), ModError> {
        let d = origin_content_descriptor(self.0)?;
        provider::exact(reg, "content", std::slice::from_ref(&d))?;
        reg.definition(d)?;
        Ok(())
    }
}
fn modules(config: Option<CoreOriginConfig>) -> Vec<Box<dyn GameModule>> {
    let cell: InventoryHandleCell = Arc::default();
    let mut industry = IndustryModule::new().with_inventory(Arc::clone(&cell));
    let mut modules: Vec<Box<dyn GameModule>> = vec![
        Box::new(PolicyOrg(OrgModule::new())),
        Box::new(EconomyModule::new()),
        Box::new(
            InventoryModule::new()
                .with_quantity_delegates(["industry"])
                .with_handle_cell(cell),
        ),
    ];
    if let Some(config) = config {
        modules.push(Box::new(Content(config.policy.grades)));
        industry = industry.with_core_origins(config);
    }
    modules.push(Box::new(industry));
    modules
}
fn command(world: &mut TestWorld, line: &str) -> Result {
    let output = world.run(GOD, line);
    if output.iter().any(|(_,e)|matches!(e,mush_proto::OutputEvent::Sys(t) if t.spans().iter().any(|s|s.tag.as_deref()==Some("error")))) {return Err(format!("actual setup command refused: {line}").into());}
    Ok(())
}
fn token(n: u8) -> CommitToken {
    CommitToken::from_bytes([n; 16])
}
fn invocation(actor: ObjId, n: u8, parameters: Value) -> ActionInvocation {
    let request = RequestUid::from_bytes([n; 16]);
    let principal = mush_core::PrincipalId::Object { id: actor };
    ActionInvocation::new(ActionInvocationDraft {
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
                owner: OwnerId::new("industry").unwrap(),
                namespace: id("origin_fixture").unwrap(),
            },
            1,
            IdempotencyKey::Request(request),
        ),
    })
    .unwrap()
}
fn fold_writes(boot: &mut Boot, writes: Vec<ReplayWrite>) {
    let ops = writes
        .into_iter()
        .map(|w| match w {
            ReplayWrite::ModuleJsonSet {
                namespace,
                key,
                value,
            } => ModuleOp {
                ns: namespace.as_str().into(),
                op: ModuleWrite::JsonSet {
                    key: key.as_str().into(),
                    value,
                },
            },
            ReplayWrite::ModuleDelete { namespace, key } => ModuleOp {
                ns: namespace.as_str().into(),
                op: ModuleWrite::Del {
                    key: key.as_str().into(),
                },
            },
            _ => panic!("Origin publication must not mutate objects, money or allocator"),
        })
        .collect();
    fold_module_ops_into(boot, ops);
}
struct Rig {
    w: TestWorld,
    runtime: Runtime,
    policy: OriginPolicy,
    boot: Boot,
}
impl Rig {
    fn new(grant: bool) -> Result<Self> {
        let mut r = Self::initialized(grant)?;
        r.activate()?;
        Ok(r)
    }
    fn activate(&mut self) -> Result {
        self.w = self.cold(self.boot.clone(), Some(CoreOriginMode::Active))?;
        // Complete the real boot write batch before preparing a new action.
        // Never drain module effects in stage/take: those must still refuse.
        self.flush();
        self.runtime = self
            .w
            .world()
            .module_state::<TestHandles>()
            .ok_or("handles")?
            .0
            .clone();
        Ok(())
    }
    fn initialized(grant: bool) -> Result<Self> {
        let mut w = TestWorld::with_modules(modules(None));
        w.world_mut().set_now(1000);
        w.world_mut().set_game_clock(mush_core::clock::GameClock {
            ic_ratio: 1,
            epoch_offset: 0,
        });
        let agent = w
            .world_mut()
            .create(ObjKind::Thing, "Configured stock agent", GOD, None)?;
        let containers = [
            w.world_mut()
                .create(ObjKind::Thing, "Importer depot", GOD, None)?,
            w.world_mut()
                .create(ObjKind::Thing, "First stock", GOD, None)?,
            w.world_mut()
                .create(ObjKind::Thing, "Second stock", GOD, None)?,
        ];
        command(&mut w, "@org/create Importer")?;
        command(&mut w, "@org/scope create Importer=die-stock")?;
        let orgs = w.world().module_state::<OrgStore>().ok_or("org")?.state();
        let importer = orgs.orgs().next().ok_or("importer")?.id;
        let scope = orgs
            .scopes_in(importer)
            .find(|s| s.name.as_str() == "die-stock")
            .ok_or("scope")?
            .id;
        for (i, c) in containers.into_iter().enumerate() {
            command(
                &mut w,
                &format!(
                    "@vault/create Stock{i}=container:#{} owner=org:Importer scope={}",
                    c.0, scope.0
                ),
            )?;
        }
        let snapshot = w
            .world()
            .module_state::<EconomyStore>()
            .ok_or("economy")?
            .snapshot();
        let find = |name: &str| {
            snapshot
                .vaults()
                .find(|v| v.name.as_str() == name)
                .map(|v| v.id)
                .ok_or("vault")
        };
        let depot = find("Stock0")?;
        let first = find("Stock1")?;
        let second = find("Stock2")?;
        let grades = [Grade::new(10)?, Grade::new(20)?, Grade::new(30)?];
        let content = DefinitionRegistryBuilder::new()
            .register(OwnerId::new("tacsim")?, origin_content_descriptor(grades)?)?;
        let policy = OriginPolicy {
            importer,
            scope,
            depot,
            agents: [
                OriginAgent {
                    actor: GOD,
                    stock: first,
                },
                OriginAgent {
                    actor: agent,
                    stock: second,
                },
            ],
            content,
            grades,
            monthly_quotas: [2, 2, 2],
        };
        if grant {
            for actor in [GOD, agent] {
                for cap in CAPS {
                    command(
                        &mut w,
                        &format!("@org/grant #{}=Importer {cap} scope=#{}", actor.0, scope.0),
                    )?;
                }
            }
        }
        let mut boot = fold_module_ops(w.take_module_ops());
        let config = CoreOriginConfig::new(policy.clone(), CoreOriginMode::Initialize)?;
        let mut w = w.reboot_into(modules(Some(config)), boot.clone())?;
        fold_module_ops_into(&mut boot, w.take_module_ops());
        let runtime = w
            .world()
            .module_state::<TestHandles>()
            .ok_or("Origin handles")?
            .0
            .clone();
        let mut r = Self {
            w,
            runtime,
            policy,
            boot,
        };
        let p = r.prepare(0, GOD, 1, json!({}))?;
        r.ack(0, GOD, 1, json!({}), p)?;
        Ok(r)
    }
    fn prepare(
        &mut self,
        phase: usize,
        actor: ObjId,
        n: u8,
        parameters: Value,
    ) -> Result<Prepared> {
        Ok(self
            .w
            .engine_mut()
            .prepare_registered_composed_authoritative_action(
                self.runtime.actions.get(phase).ok_or("phase")?,
                &invocation(actor, n, parameters),
                token(n),
            )?)
    }
    fn stage(
        &mut self,
        phase: usize,
        actor: ObjId,
        n: u8,
        parameters: Value,
        p: Prepared,
    ) -> Result {
        let inv = invocation(actor, n, parameters);
        let action = self.runtime.actions.get(phase).ok_or("phase")?.clone();
        let (
            outcome,
            resolution,
            mut definitions,
            eligible_witnesses,
            durable_outbox,
            semantic_events,
        ) = p.1.into_parts();
        definitions.push(action.clone());
        definitions.sort();
        definitions.dedup();
        let bundle = AuthoritativeBundle::new(AuthoritativeBundleDraft {
            token: token(n),
            request: inv.request(),
            idempotency: inv.idempotency().clone(),
            ingress_hash: CanonicalHash::from_bytes([1; 32]),
            request_hash: inv.request_hash(),
            plan_hash: CanonicalHash::from_bytes([3; 32]),
            action,
            provider: OwnerId::new("industry")?,
            cause: PreparedCausalRef::new(inv.request(), None, 0)?,
            definitions,
            writes: p.0.authoritative_replay_writes(),
            outcome,
            resolution: resolution.map(|r| r.bind(token(n))),
            semantic_events,
            eligible_witnesses,
            durable_outbox,
        })?;
        let guards = self
            .w
            .engine()
            .registry
            .server_guard_set_builder(mush_core::RevisionSet::default())?
            .finish()?;
        self.w.world_mut().take_dirty();
        self.w
            .engine_mut()
            .stage_authoritative_action(p.0, guards, bundle)?;
        Ok(())
    }
    fn ack(&mut self, phase: usize, actor: ObjId, n: u8, parameters: Value, p: Prepared) -> Result {
        self.stage(phase, actor, n, parameters, p)?;
        let held = self
            .w
            .engine_mut()
            .take_staged_action()?
            .ok_or("staged action")?;
        let (bundle, install) = held.into_parts();
        self.w.engine_mut().install_composed_action(install)?;
        fold_writes(&mut self.boot, bundle.writes().to_vec());
        Ok(())
    }
    fn issue(&mut self, actor: ObjId, n: u8, class: DieClass) -> Result<DieCertificate> {
        let parameters = json!({"class":class});
        let p = self.prepare(1, actor, n, parameters.clone())?;
        self.ack(1, actor, n, parameters, p)?;
        Ok(self.cert(n)?.clone())
    }
    fn cert(&self, n: u8) -> Result<&DieCertificate> {
        Ok(self
            .w
            .engine()
            .action_state()
            .get(&self.runtime.families.certificates, &token(n).to_hex())?
            .ok_or("certificate")?)
    }
    fn issued(&self) -> u64 {
        self.w
            .world()
            .module_state::<CoreProductionState>()
            .unwrap()
            .index
            .as_ref()
            .unwrap()
            .issued
    }
    fn flush(&mut self) {
        fold_module_ops_into(&mut self.boot, self.w.take_module_ops());
    }
    fn cold(&self, boot: Boot, mode: Option<CoreOriginMode>) -> Result<TestWorld> {
        let config = mode
            .map(|m| CoreOriginConfig::new(self.policy.clone(), m))
            .transpose()?;
        let (_, registry) = mush_testkit::boot_world();
        let mut e = Engine::with_modules(self.w.world().clone(), registry, modules(config))?;
        // As in the real server loader, hydrate the acknowledged typed envelopes
        // before ordinary owner hooks receive their independent documents.
        let mut boot = boot;
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
        e.hydrate_action_state(typed)?;
        e.modules_loaded(
            boot,
            &mut Outbox::new(),
            &mut mush_core::engine::Effects::new(),
        )?;
        Ok(TestWorld::from_engine(e))
    }
    fn deny(&mut self, actor: ObjId) -> Result {
        command(
            &mut self.w,
            &format!("@origin-policy {} {} deny", actor.0, self.policy.scope.0),
        )?;
        self.flush();
        Ok(())
    }
}
#[test]
fn actual_origin_issuance_is_atomic_and_replays_exact_inventory_and_certificate() -> Result {
    let mut r = Rig::new(true)?;
    let actor = r.policy.agents[1].actor;
    let before = r.w.engine().action_state().revision();
    let allocator = r.w.world().next_id();
    let p = r.prepare(1, actor, 2, json!({"class":"i"}))?;
    assert_eq!(r.issued(), 0);
    assert_eq!(r.w.engine().action_state().revision(), before);
    assert!(p.0.authoritative_replay_writes().iter().all(|w|matches!(w,ReplayWrite::ModuleJsonSet{namespace,..} if ["industry","inventory"].contains(&namespace.as_str()))));
    r.ack(1, actor, 2, json!({"class":"i"}), p)?;
    let c = r.cert(2)?;
    assert_eq!(c.lot, lot_id(token(2))?);
    assert_eq!(c.source, source(token(2)));
    assert_eq!(c.origin_endpoint.0, r.policy.agents[1].stock);
    let lot =
        r.w.engine()
            .action_state()
            .get(&r.runtime.families.inventory.lots, &c.lot)?
            .ok_or("lot")?;
    c.matches_lot(lot)?;
    assert!(matches!(lot.lineage(), Lineage::Origin { .. }));
    assert_eq!(lot.quantity(), 1);
    assert_eq!(r.w.world().next_id(), allocator);
    assert_eq!(r.issued(), 1);
    let cold = r.cold(r.boot.clone(), Some(CoreOriginMode::Active))?;
    let h = &cold
        .world()
        .module_state::<TestHandles>()
        .ok_or("handles")?
        .0;
    assert_eq!(
        cold.engine()
            .action_state()
            .get(&h.families.certificates, &token(2).to_hex())?,
        Some(c)
    );
    assert!(r.cold(r.boot.clone(), None).is_err());
    Ok(())
}
#[test]
fn monthly_quota_is_shared_by_agents_and_survives_rate_changes_rewinds_and_restart() -> Result {
    let mut r = Rig::new(true)?;
    let second = r.policy.agents[1].actor;
    r.issue(GOD, 2, DieClass::I)?;
    r.issue(second, 3, DieClass::I)?;
    assert!(r.prepare(1, GOD, 4, json!({"class":"i"})).is_err());
    let epoch = r.w.world().next_world_timeline_epoch(2)?;
    r.w.world_mut().append_world_timeline_epoch(epoch)?;
    assert!(r.prepare(1, second, 5, json!({"class":"i"})).is_err());
    r.w.world_mut().set_now(900);
    assert!(r.prepare(1, GOD, 6, json!({"class":"i"})).is_err());
    r.activate()?;
    assert!(r.prepare(1, GOD, 7, json!({"class":"i"})).is_err());
    r.w.world_mut().set_now(40 * 86400);
    let next = r.issue(GOD, 8, DieClass::I)?;
    assert_eq!(next.ordinal, 1);
    assert_eq!(r.issued(), 3);
    r.cold(r.boot.clone(), Some(CoreOriginMode::Active))?;
    Ok(())
}
#[test]
fn explicit_scoped_policy_is_required_even_for_staff_and_deny_precedes_allow() -> Result {
    let mut r = Rig::new(false)?;
    assert!(r.prepare(1, GOD, 2, json!({"class":"i"})).is_err());
    command(
        &mut r.w,
        &format!("@origin-policy {} {} allow", GOD.0, r.policy.scope.0),
    )?;
    r.flush();
    r.issue(GOD, 3, DieClass::I)?;
    r.deny(GOD)?;
    assert!(r.prepare(1, GOD, 4, json!({"class":"ii"})).is_err());
    assert_eq!(r.issued(), 1);
    r.cold(r.boot.clone(), Some(CoreOriginMode::Active))?;
    Ok(())
}
#[test]
fn authority_vault_month_and_clock_changes_refuse_before_stage_and_take() -> Result {
    for mutation in 0..4 {
        for after_stage in [false, true] {
            let mut r = Rig::new(true)?;
            let p = r.prepare(1, GOD, 2, json!({"class":"i"}))?;
            let before = r.w.engine().action_state().revision();
            let p = if after_stage {
                r.stage(1, GOD, 2, json!({"class":"i"}), p)?;
                None
            } else {
                Some(p)
            };
            match mutation {
                0 => r.deny(GOD)?,
                1 => {
                    command(
                        &mut r.w,
                        &format!("@vault/freeze {}", r.policy.agents[0].stock),
                    )?;
                    r.flush();
                }
                2 => r.w.world_mut().set_now(40 * 86400),
                _ => {
                    let epoch = r.w.world().next_world_timeline_epoch(2)?;
                    r.w.world_mut().append_world_timeline_epoch(epoch)?;
                }
            }
            if let Some(p) = p {
                assert!(r.stage(1, GOD, 2, json!({"class":"i"}), p).is_err());
            } else {
                assert!(r.w.engine_mut().take_staged_action().is_err());
            }
            assert_eq!(r.issued(), 0);
            assert_eq!(r.w.engine().action_state().revision(), before);
        }
    }
    Ok(())
}
#[test]
fn cold_reverse_audit_rejects_missing_and_crossed_halves_including_all_typed_loss() -> Result {
    let mut r = Rig::new(true)?;
    r.issue(GOD, 2, DieClass::I)?;
    for missing in [
        "industry_core_die_certificates",
        "industry_core_die_quota",
        "industry_core_die_deployment",
        INDEX_KEY,
        "all_typed",
    ] {
        let mut boot = r.boot.clone();
        let data = boot.get_mut("industry").ok_or("industry boot")?;
        let before = data.kv.len();
        data.kv.retain(|(k, v)| {
            if missing == INDEX_KEY {
                k.as_str() != INDEX_KEY
            } else if missing == "all_typed" {
                v.get("family").is_none()
            } else {
                v.get("family")
                    .and_then(|f| f.get("id"))
                    .and_then(Value::as_str)
                    != Some(missing)
            }
        });
        assert!(data.kv.len() < before, "exact corruption target exists");
        assert!(
            r.cold(boot, Some(CoreOriginMode::Active)).is_err(),
            "incomplete retained Origin evidence refused"
        );
    }
    for field in ["lot", "agent", "ordinal", "source"] {
        let mut boot = r.boot.clone();
        let data = boot.get_mut("industry").ok_or("industry boot")?;
        let row = data
            .kv
            .iter_mut()
            .map(|(_, v)| v)
            .find(|v| {
                v.get("family")
                    .and_then(|f| f.get("id"))
                    .and_then(Value::as_str)
                    == Some("industry_core_die_certificates")
            })
            .ok_or("certificate envelope")?;
        let value = row.get_mut("value").ok_or("certificate value")?;
        let replacement = match field {
            "lot" => serde_json::to_value(lot_id(token(99))?)?,
            "agent" => json!(r.policy.agents[1].actor),
            "ordinal" => json!(2),
            _ => serde_json::to_value(source(token(99)))?,
        };
        value
            .as_object_mut()
            .ok_or("certificate object")?
            .insert(field.into(), replacement);
        assert!(
            r.cold(boot, Some(CoreOriginMode::Active)).is_err(),
            "crossed issuance evidence refused"
        );
    }
    Ok(())
}
#[test]
fn issue_request_is_closed_and_cannot_select_agent_vault_quantity_grade_or_source() -> Result {
    let mut r = Rig::new(true)?;
    for value in [
        json!({}),
        json!({"class":"zero"}),
        json!({"class":"i","actor":GOD}),
        json!({"class":"i","vault":1}),
        json!({"class":"i","quantity":2}),
        json!({"class":"i","grade":10}),
        json!({"class":"i","source":null}),
    ] {
        assert!(r.prepare(1, GOD, 2, value).is_err());
    }
    let stranger = r.w.spawn_player("Unconfigured requester");
    assert!(r.prepare(1, stranger, 3, json!({"class":"i"})).is_err());
    assert_eq!(r.issued(), 0);
    Ok(())
}
#[test]
fn actual_historical_origin_can_retire_but_neither_pool_nor_new_lot_inherits_certificate() -> Result
{
    let mut r = Rig::new(true)?;
    let c = r.issue(GOD, 2, DieClass::I)?;
    let f = r.runtime.families.clone();
    let prepared = {
        let mut txn = r.w.engine_mut().action_transaction(&f.certificates)?;
        f.inventory.dissolve(&mut txn, c.origin_endpoint, c.lot)?;
        txn.prepare()?
    };
    let (writes, install) = prepared.into_parts();
    r.w.engine_mut().install_prepared_action(install)?;
    fold_module_ops_into(&mut r.boot, writes);
    assert!(
        r.w.engine()
            .action_state()
            .get(&f.inventory.lots, &c.lot)?
            .is_none()
    );
    assert!(r.cert(2).is_ok());
    r.cold(r.boot.clone(), Some(CoreOriginMode::Active))?;
    Ok(())
}

#[test]
fn dependency_mechanics_freeze_refuses_each_actual_owner_before_custody() -> Result {
    for owner in ["industry", "org", "economy", "inventory", "tacsim"] {
        for after_stage in [false, true] {
            let mut r = Rig::new(true)?;
            let p = r.prepare(1, GOD, 2, json!({"class":"i"}))?;
            let p = if after_stage {
                r.stage(1, GOD, 2, json!({"class":"i"}), p)?;
                None
            } else {
                Some(p)
            };
            r.w.engine().registry.freeze_mechanics_owner(
                &OwnerId::new(owner)?,
                mush_core::mechanics::IncidentId(1),
                mush_core::mechanics::RecoveryReason::IntegrityViolation {
                    code: id("origin_fixture")?,
                },
            )?;
            if let Some(p) = p {
                assert!(r.stage(1, GOD, 2, json!({"class":"i"}), p).is_err());
            } else {
                assert!(r.w.engine_mut().take_staged_action().is_err());
            }
            assert_eq!(r.issued(), 0);
        }
    }
    Ok(())
}
#[test]
fn acknowledged_issuance_keeps_its_original_month_when_the_next_month_arrives() -> Result {
    let mut r = Rig::new(true)?;
    let p = r.prepare(1, GOD, 2, json!({"class":"i"}))?;
    r.stage(1, GOD, 2, json!({"class":"i"}), p)?;
    let held =
        r.w.engine_mut()
            .take_staged_action()?
            .ok_or("actual custody")?;
    let old = r.w.world().world_ic_now();
    r.w.world_mut().set_now(40 * 86400);
    let (bundle, install) = held.into_parts();
    r.w.engine_mut().install_composed_action(install)?;
    fold_writes(&mut r.boot, bundle.writes().to_vec());
    assert_eq!(r.cert(2)?.observed_ic, old);
    assert_ne!(
        ImportMonth::from_stamp(&old)?,
        ImportMonth::from_stamp(&r.w.world().world_ic_now())?
    );
    r.cold(r.boot.clone(), Some(CoreOriginMode::Active))?;
    Ok(())
}
#[test]
fn actual_inventory_origin_claim_without_matching_certificate_is_a_cold_refusal() -> Result {
    let mut r = Rig::new(true)?;
    let f = r.runtime.families.clone();
    let endpoint = mush_inventory::EndpointRef(r.policy.agents[0].stock);
    let prepared = {
        let mut txn = r.w.engine_mut().action_transaction(&f.certificates)?;
        f.inventory.mint_origin(
            &mut txn,
            endpoint,
            &id("tacsim.electronics")?,
            1,
            r.policy.grades[0],
            source(token(99)),
            lot_id(token(99))?,
        )?;
        txn.prepare()?
    };
    let (writes, install) = prepared.into_parts();
    r.w.engine_mut().install_prepared_action(install)?;
    fold_module_ops_into(&mut r.boot, writes);
    assert_eq!(r.issued(), 0);
    assert!(
        r.cold(r.boot.clone(), Some(CoreOriginMode::Active))
            .is_err()
    );
    Ok(())
}
#[test]
fn a_higher_current_configuration_does_not_expand_an_existing_months_cap() -> Result {
    let mut r = Rig::new(true)?;
    r.issue(GOD, 2, DieClass::I)?;
    r.issue(GOD, 3, DieClass::I)?;
    r.policy.monthly_quotas = [1024; 3];
    r.activate()?;
    assert!(r.prepare(1, GOD, 4, json!({"class":"i"})).is_err());
    r.w.world_mut().set_now(40 * 86400);
    let c = r.issue(GOD, 5, DieClass::I)?;
    assert_eq!(c.quota_maximum, 1024);
    r.cold(r.boot.clone(), Some(CoreOriginMode::Active))?;
    Ok(())
}

#[test]
fn acknowledged_initialization_cannot_issue_until_actual_active_cold_boot() -> Result {
    let mut r = Rig::initialized(true)?;
    assert_eq!(r.runtime.config.mode, CoreOriginMode::Initialize);
    for actor in [GOD, r.policy.agents[1].actor] {
        assert!(r.prepare(1, actor, 2, json!({"class":"i"})).is_err());
    }
    assert_eq!(r.issued(), 0);
    assert!(r.prepare(0, GOD, 3, json!({})).is_err());
    r.activate()?;
    assert_eq!(r.runtime.config.mode, CoreOriginMode::Active);
    assert!(r.prepare(0, GOD, 4, json!({})).is_err());
    r.issue(r.policy.agents[1].actor, 5, DieClass::I)?;
    assert_eq!(r.issued(), 1);
    Ok(())
}

#[path = "integration_tests.rs"]
mod factory_integration;

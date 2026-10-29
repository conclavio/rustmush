use super::*;
use super::{
    authority::Definitions,
    participant::{CoreProductionState, OriginParticipant, Readiness, Request},
    records::*,
};
use mush_core::action::{ActionError, ActionErrorCode, ActionProvider, AuthoritativeActionOutput};
use mush_core::mechanics::{DefinitionKind, DefinitionValidationContext, OwnerId, RulesValue};
use mush_core::participant::ParticipantHandle;
use mush_core::read_service::OwnerReadHandle;
use mush_core::{
    ActionInvocation, ActionTxn, ModuleBootData, ModuleCtx, PerceivedRef, PrepareCtx, Registrar,
    StateFamilyDescriptor, StateFamilyHandle, StateFamilyId,
};
use mush_inventory::{EndpointRef, InventoryFamilies};
use serde::Deserialize;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone)]
pub(crate) struct Families {
    pub(crate) certificates: StateFamilyHandle<String, DieCertificate>,
    pub(crate) quotas: StateFamilyHandle<String, Quota>,
    pub(crate) deployment: StateFamilyHandle<(), OriginIndex>,
    pub(crate) inventory: InventoryFamilies,
    /// Doc 17h cut M8b-2 (H3): the heirloom and grey issues.
    pub(crate) uncertified: super::uncertified::Families,
}
#[derive(Clone)]
pub(crate) struct Runtime {
    pub(crate) config: CoreOriginConfig,
    pub(crate) families: Families,
    pub(crate) definitions: Definitions,
    pub(crate) actions: [DefinitionRef; 2],
    pub(crate) integration: Option<super::integration::runtime::Runtime>,
    pub(crate) refit: Option<super::refit::runtime::Runtime>,
}
#[cfg(test)]
#[derive(Clone)]
pub(super) struct TestHandles(pub Box<Runtime>);
impl CoreOriginConfig {
    pub(crate) fn outcome_descriptor(
        &self,
    ) -> Result<mush_core::mechanics::DefinitionDescriptor, ModError> {
        config::descriptor(
            "outcome",
            "core_origin_issued",
            json!({"format":1,"fields":["operation","lot","class"],"initialize_lot":null}),
        )
    }
}
fn family(name: &str) -> Result<StateFamilyDescriptor, ModError> {
    let cid = |s: &str| CanonicalId::new(format!("industry.{name}.{s}.v1")).map_err(mod_error);
    StateFamilyDescriptor::new(
        StateFamilyId::new(name).map_err(mod_error)?,
        cid("key")?,
        cid("value")?,
        1,
        cid("decode")?,
        cid("validate")?,
        cid("canonical")?,
    )
    .and_then(|d| d.with_limits(96, RECORD_BYTES + 1024))
    .map_err(mod_error)
}
pub(crate) fn exact(
    reg: &mut Registrar<'_>,
    kind: &str,
    descriptors: &[mush_core::mechanics::DefinitionDescriptor],
) -> Result<(), ModError> {
    let values: BTreeMap<_, _> = descriptors
        .iter()
        .map(|d| {
            let mut references = d.references().to_vec();
            references.sort();
            (
                (d.id().clone(), d.version()),
                (d.payload().clone(), references, d.is_current()),
            )
        })
        .collect();
    reg.definition_validator(
        DefinitionKind::new(kind).map_err(mod_error)?,
        move |ctx: &DefinitionValidationContext<'_>| {
            let e = ctx.definition();
            match values.get(&(e.reference().key().id.clone(), e.reference().version())) {
                Some((p, r, c))
                    if e.payload() == p && e.references() == r && e.is_current() == *c =>
                {
                    Ok(())
                }
                _ => Err("invalid certified Origin definition".into()),
            }
        },
    )
}
#[inline(never)]
fn register_origin_owner(
    reg: &mut Registrar<'_>,
    config: &CoreOriginConfig,
    definitions: &Definitions,
    actions: &[DefinitionRef; 2],
    integration: &Option<super::integration::runtime::Runtime>,
    refit: &Option<super::refit::runtime::Runtime>,
) -> Result<DefinitionRef, ModError> {
    let mut refs = actions.to_vec();
    refs.push(config.policy.content.clone());
    refs.sort();
    refs.dedup();
    let pd=config::descriptor("owner_participant","core_production",json!({"format":1,"protocol":"certified_origin_stock_v1","index":INDEX_KEY,"actions":actions,"content":config.policy.content,"max_certificates":MAX_CERTIFICATES}))?.with_references(refs.clone());
    let pd = match integration {
        Some(r) => r
            .config
            .participant_descriptor(&config.policy)
            .map_err(mod_error)?,
        None => pd,
    };
    exact(reg, "owner_participant", std::slice::from_ref(&pd))?;
    let participant = reg.definition(pd)?;
    let mut allowed: BTreeSet<_> = actions.iter().cloned().collect();
    if let Some(r) = integration {
        allowed.extend(
            r.actions
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != 3)
                .map(|(_, a)| a.clone()),
        );
    }
    if let Some(d) = integration.as_ref().and_then(|r| r.delivery.as_ref()) {
        allowed.insert(d.action.clone());
    }
    if let Some(r) = refit {
        allowed.extend(
            r.actions
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != 4)
                .map(|(_, a)| a.clone()),
        );
    }
    reg.owner_participant(
        participant.clone(),
        allowed,
        OriginParticipant {
            config: config.clone(),
            definitions: definitions.clone(),
            actions: actions.clone(),
            integration: integration
                .as_ref()
                .map(|r| (r.config.clone(), r.inputs.clone())),
        },
    )?;
    Ok(participant)
}

#[inline(never)]
fn register_origin_readiness(
    reg: &mut Registrar<'_>,
    config: &CoreOriginConfig,
    integration: &Option<super::integration::runtime::Runtime>,
    refit: &Option<super::refit::runtime::Runtime>,
) -> Result<OwnerReadHandle<Readiness>, ModError> {
    let rd=config::descriptor("owner_read","core_production_readiness",json!({"format":1,"protocol":"certified_origin_dependencies_v1","content":config.policy.content}))?.with_references(vec![config.policy.content.clone()]);
    let mut reads = vec![rd.clone()];
    if let Some(r) = integration {
        reads.push(super::integration::runtime::output_binding(&r.config)?.policy_descriptor()?);
    }
    if let Some(d) = integration.as_ref().and_then(|r| r.delivery.as_ref()) {
        reads.push(mush_core::core_delivery::CoreOutputDeliveries::<
            super::integration::output_policy::Policy,
            super::integration::delivery::Policy,
        >::policy_descriptor(&d.action)?);
    }
    if let Some(r) = refit {
        reads.push(mush_core::core_exchange::CoreComponentExchanges::<
            super::refit::policies::ExchangePolicy,
        >::policy_descriptor(&r.actions[4], &r.participants)?);
    }
    exact(reg, "owner_read", &reads)?;
    let rd = reg.definition(rd)?;
    let industry = OwnerId::new(crate::NS).map_err(mod_error)?;
    let mut dependencies = BTreeSet::from([
        industry.clone(),
        OwnerId::new("org").map_err(mod_error)?,
        OwnerId::new("economy").map_err(mod_error)?,
        OwnerId::new("inventory").map_err(mod_error)?,
    ]);
    dependencies.insert(config.policy.content.key().owner.clone());
    reg.owner_read_service(
        rd.clone(),
        BTreeSet::from([industry]),
        dependencies,
        Readiness,
    )?;
    let readiness = reg.request_owner_read::<Readiness>(rd)?;
    Ok(readiness)
}

#[inline(never)]
fn register_origin_paths(
    reg: &mut Registrar<'_>,
    runtime: &Runtime,
    outcome: DefinitionRef,
    participant: DefinitionRef,
    readiness: OwnerReadHandle<Readiness>,
) -> Result<(), ModError> {
    for initialize in [true, false] {
        let action = runtime.actions[if initialize { 0 } else { 1 }].clone();
        let owner = reg.request_owner_participant::<OriginParticipant>(action.clone())?;
        reg.action_provider(
            action,
            Provider {
                runtime: Box::new(runtime.clone()),
                initialize,
                outcome: outcome.clone(),
                owner,
                readiness: readiness.clone(),
                refusal: refusal()?,
            },
        )?;
    }
    // Doc 17h cut M8b-2 (H3): the staff issues of heirloom and grey dies, beside the
    // certified issue and outside its owner participant and quota.
    for (name, source) in super::uncertified::ACTIONS {
        let action = reg.definition(super::uncertified::action(&runtime.config.policy, source)?)?;
        if !fixed(&action, "action", name) {
            return Err(mod_error(Error::Invalid));
        }
        reg.action_provider(
            action.clone(),
            super::uncertified::Provider {
                source,
                action,
                outcome: outcome.clone(),
                policy: runtime.config.policy.clone(),
                mode: runtime.config.mode,
                families: runtime.families.uncertified.clone(),
                inventory: runtime.families.inventory.clone(),
                readiness: readiness.clone(),
                refusal: super::uncertified::refusal()?,
            },
        )?;
    }
    if let Some(r) = &runtime.integration {
        if let Some(delivery) = &r.delivery {
            reg.core_integration_delivery_capacity(
                [participant.clone(), r.config.economy_owner.clone()],
                [
                    r.actions[2].clone(),
                    r.actions[3].clone(),
                    r.actions[4].clone(),
                    r.actions[5].clone(),
                    r.actions[6].clone(),
                    delivery.action.clone(),
                ],
            )?;
        } else {
            reg.core_integration_capacity(
                [participant.clone(), r.config.economy_owner.clone()],
                [
                    r.actions[2].clone(),
                    r.actions[3].clone(),
                    r.actions[4].clone(),
                    r.actions[5].clone(),
                    r.actions[6].clone(),
                ],
            )?;
        }
        super::integration::provider::register(reg, r.clone(), readiness.clone())?;
        super::integration::schedule::register(reg, r.clone())?;
    }
    if let Some(r) = &runtime.refit {
        reg.core_refit_capacity(
            [
                participant,
                r.participants[0].clone(),
                r.participants[1].clone(),
            ],
            [
                r.actions[2].clone(),
                r.actions[3].clone(),
                r.actions[4].clone(),
                r.actions[5].clone(),
            ],
        )?;
        super::refit::provider::register(reg, r.clone(), readiness)?;
        super::refit::schedule::register(reg, r.clone())?;
        let hook = std::sync::Arc::new(super::refit::policies::CustodyHook {
            config: r.config.clone(),
            families: r.families.clone(),
            participants: r.participants.clone(),
        });
        reg.hook(mush_core::HookPoint::CarryAttempt, hook.clone());
        reg.hook(mush_core::HookPoint::MoveAttempt, hook);
    }
    Ok(())
}

#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn finish_origin_registration(
    reg: &mut Registrar<'_>,
    config: CoreOriginConfig,
    families: Families,
    definitions: Definitions,
    actions: [DefinitionRef; 2],
    integration: Option<super::integration::runtime::Runtime>,
    refit: Option<super::refit::runtime::Runtime>,
    outcome: DefinitionRef,
) -> Result<Box<Runtime>, ModError> {
    let participant =
        register_origin_owner(reg, &config, &definitions, &actions, &integration, &refit)?;
    let readiness = register_origin_readiness(reg, &config, &integration, &refit)?;
    reg.state(CoreProductionState::default());
    let runtime = Box::new(Runtime {
        config,
        families,
        integration,
        refit,
        definitions,
        actions,
    });
    #[cfg(test)]
    reg.state(TestHandles(runtime.clone()));
    register_origin_paths(reg, &runtime, outcome, participant, readiness)?;
    Ok(runtime)
}

impl Runtime {
    pub(crate) fn register(
        reg: &mut Registrar<'_>,
        config: CoreOriginConfig,
        industry_families: crate::IndustryFamilies,
        factory: Option<super::integration::CoreFactoryConfig>,
    ) -> Result<Box<Self>, ModError> {
        if reg.module() != crate::NS {
            return Err(mod_error(Error::Invalid));
        }
        config.policy.validate().map_err(mod_error)?;
        let actions = [
            reg.definition(config.policy.action(true)?)?,
            reg.definition(config.policy.action(false)?)?,
        ];
        let outcome = reg.definition(config.outcome_descriptor()?)?;
        let certificates = reg.state_family(
            family("industry_core_die_certificates")?,
            |s: &String| Ok(s.clone()),
            decode::<DieCertificate>,
            |r: &DieCertificate| r.validate().map_err(|e| e.to_string()),
            encode::<DieCertificate>,
        )?;
        if let Some(reader) = config.certificate_reader.as_deref() {
            reg.grant_state_family_read(&certificates, reader)?;
            reg.state(super::records::DieCertificates(certificates.clone()));
        }
        let quotas = reg.state_family(
            family("industry_core_die_quota")?,
            |s: &String| Ok(s.clone()),
            decode::<Quota>,
            |r: &Quota| r.validate().map_err(|e| e.to_string()),
            encode::<Quota>,
        )?;
        let uncertified = super::uncertified::register_families(
            reg,
            config.certificate_reader.as_deref(),
            family,
        )?;
        let deployment = reg.state_family(
            family("industry_core_die_deployment")?,
            |_: &()| Ok("registry".into()),
            decode::<OriginIndex>,
            |r: &OriginIndex| r.validate().map_err(|e| e.to_string()),
            encode::<OriginIndex>,
        )?;
        let definitions = Definitions::default();
        let families = Families {
            certificates,
            quotas,
            deployment,
            inventory: industry_families.inventory.clone(),
            uncertified,
        };
        Self::register_factory(
            reg,
            config,
            industry_families,
            factory,
            families,
            definitions,
            actions,
            outcome,
        )
    }

    /// **The factory's integration and refit owners, then the origin's finish** (kept out of
    /// `register`'s frame: two runtimes and their policies — doc 17h cut M8b's refit policy
    /// carries the rig families and the ungoverning site — exceed the M7 memory-size gate
    /// there).
    #[inline(never)]
    #[allow(clippy::too_many_arguments)]
    fn register_factory(
        reg: &mut Registrar<'_>,
        config: CoreOriginConfig,
        industry_families: crate::IndustryFamilies,
        factory: Option<super::integration::CoreFactoryConfig>,
        families: Families,
        definitions: Definitions,
        actions: [DefinitionRef; 2],
        outcome: DefinitionRef,
    ) -> Result<Box<Self>, ModError> {
        let integration = factory
            .map(|factory| {
                super::integration::runtime::Runtime::register(
                    reg,
                    factory,
                    industry_families,
                    families.clone(),
                    definitions.clone(),
                )
            })
            .transpose()?;
        let refit = integration
            .as_ref()
            .and_then(|r| r.config.refit.clone())
            .map(|c| {
                let r = integration
                    .as_ref()
                    .ok_or_else(|| mod_error(Error::Integrity))?;
                super::refit::runtime::Runtime::register(
                    reg,
                    c,
                    r.industry.clone(),
                    r.config.economy_owner.clone(),
                    r.families.clone(),
                )
            })
            .transpose()?;
        finish_origin_registration(
            reg,
            config,
            families,
            definitions,
            actions,
            integration,
            refit,
            outcome,
        )
    }
    pub(crate) fn load(
        &self,
        ctx: &mut ModuleCtx<'_>,
        data: &ModuleBootData,
    ) -> Result<(), ModError> {
        let defs = ctx
            .reg
            .definitions()
            .ok_or_else(|| mod_error(Error::Unavailable))?
            .clone();
        self.definitions
            .0
            .set(defs.clone())
            .map_err(|_| mod_error(Error::Integrity))?;
        if let Some(r) = &self.refit {
            r.config
                .definitions
                .0
                .set(defs)
                .map_err(|_| mod_error(Error::Integrity))?;
        }
        if data
            .kv
            .iter()
            .filter(|(k, _)| k.as_str() == INDEX_KEY)
            .count()
            > 1
        {
            return Err(mod_error(Error::Integrity));
        }
        let index = data
            .kv
            .iter()
            .find(|(k, _)| k.as_str() == INDEX_KEY)
            .map(|(_, v)| decode::<OriginIndex>(v))
            .transpose()
            .map_err(mod_error)?;
        let refit_index = super::refit::audit::load_index(data).map_err(mod_error)?;
        if self.refit.is_none() && refit_index.is_some() {
            return Err(mod_error(
                "Core refit history requires its configured owner",
            ));
        }
        let integration_index = super::integration::audit::load_index(data).map_err(mod_error)?;
        let delivery_index = super::integration::delivery::audit::load(data).map_err(mod_error)?;
        if delivery_index.is_some()
            && self
                .integration
                .as_ref()
                .and_then(|r| r.delivery.as_ref())
                .is_none()
        {
            return Err(mod_error(
                "Core delivery history requires its configured owner",
            ));
        }
        if self.integration.is_none() && integration_index.is_some() {
            return Err(mod_error(
                "Core integration history requires its configured owner",
            ));
        }
        let state = ctx
            .w
            .module_state_mut::<CoreProductionState>()
            .ok_or_else(|| mod_error(Error::Unavailable))?;
        state.ready = false;
        state.index = index;
        state.generation = 1;
        state.refit = self
            .refit
            .as_ref()
            .map(|_| super::refit::state::Projection {
                index: refit_index,
                ..Default::default()
            });
        state.integration =
            self.integration
                .as_ref()
                .map(|_| super::integration::state::Projection {
                    index: integration_index,
                    delivery_index,
                    ..Default::default()
                });
        Ok(())
    }
    pub(crate) fn finalize(&self, ctx: &mut ModuleCtx<'_>) -> Result<(), ModError> {
        audit::audit(ctx, self).map_err(mod_error)?;
        let integration = match &self.integration {
            Some(r) => Some(super::integration::audit::audit(ctx, r).map_err(mod_error)?),
            None => {
                ctx.w
                    .module_state::<mush_economy::EconomyStore>()
                    .ok_or_else(|| mod_error(Error::Unavailable))?
                    .require_no_core_payment_history()
                    .map_err(mod_error)?;
                None
            }
        };
        let refit = match &self.refit {
            Some(r) => Some(super::refit::audit::audit(ctx, r).map_err(mod_error)?),
            None => {
                ctx.w
                    .module_state::<mush_economy::EconomyStore>()
                    .ok_or_else(|| mod_error(Error::Unavailable))?
                    .require_no_core_refit_history()
                    .map_err(mod_error)?;
                None
            }
        };
        let state = ctx
            .w
            .module_state_mut::<CoreProductionState>()
            .ok_or_else(|| mod_error(Error::Unavailable))?;
        state.refit = refit;
        state.integration = integration;
        state.ready = true;
        Ok(())
    }
}
fn refusal() -> Result<ActionError, ModError> {
    ActionError::new(
        ActionErrorCode::ProviderRejected,
        false,
        "Certified core die stock is unavailable or changed.",
    )
    .map_err(mod_error)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IssueRequest {
    class: DieClass,
}
fn parse(inv: &ActionInvocation, initialize: bool) -> Result<Option<DieClass>, Error> {
    if inv.principal() != &(mush_core::action::PrincipalId::Object { id: inv.actor() })
        || inv.subject() != &PerceivedRef::SelfActor
        || !inv.targets().is_empty()
        || inv.idempotency().scope().owner.as_str() != crate::NS
    {
        return Err(Error::Invalid);
    }
    if initialize {
        if inv.parameters() != &json!({}) {
            return Err(Error::Invalid);
        }
        Ok(None)
    } else {
        serde_json::from_value::<IssueRequest>(inv.parameters().clone())
            .map(|r| Some(r.class))
            .map_err(bad)
    }
}
struct Provider {
    runtime: Box<Runtime>,
    initialize: bool,
    outcome: DefinitionRef,
    owner: ParticipantHandle<OriginParticipant>,
    readiness: OwnerReadHandle<Readiness>,
    refusal: ActionError,
}
impl Provider {
    fn prepare(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, Error> {
        let class = parse(inv, self.initialize)?;
        if class.is_some() && self.runtime.config.mode != CoreOriginMode::Active {
            return Err(Error::Unavailable);
        }
        ctx.read_owner_preimage(txn, &self.readiness, ())
            .map_err(bad)?;
        let f = &self.runtime.families;
        let before = txn.get(&f.deployment, &()).map_err(bad)?.cloned();
        let operation = ctx.commit_token();
        let (request, certificate, quota) = if let Some(class) = class {
            let index = before.as_ref().ok_or(Error::Unavailable)?;
            index.validate()?;
            if index.issued >= MAX_CERTIFICATES
                || txn
                    .get(&f.certificates, &operation.to_hex())
                    .map_err(bad)?
                    .is_some()
            {
                return Err(Error::Capacity);
            }
            let live = authority::check(
                ctx.world().map_err(bad)?,
                &self.runtime.definitions,
                &self.runtime.config.policy,
                inv.actor(),
            )?;
            let key = QuotaKey {
                importer: self.runtime.config.policy.importer,
                month: ImportMonth::from_stamp(&live.stamp)?,
                class,
            };
            let old = txn.get(&f.quotas, &key.key()?).map_err(bad)?.cloned();
            let quota = Quota::issue(
                key,
                self.runtime.config.policy.monthly_quotas[class.index()],
                old.as_ref(),
                operation,
            )?;
            let certificate = DieCertificate {
                format: 1,
                sequence: index.issued.checked_add(1).ok_or(Error::Capacity)?,
                operation,
                action: self.runtime.actions[1].clone(),
                policy: self.runtime.config.policy.clone(),
                agent: inv.actor(),
                class,
                label: id(class.label())?,
                lot: lot_id(operation)?,
                commodity: id("tacsim.electronics")?,
                grade: self.runtime.config.policy.grades[class.index()],
                source: source(operation),
                origin_endpoint: EndpointRef(self.runtime.config.policy.agent(inv.actor())?.stock),
                observed_ic: live.stamp,
                observed_utc: live.utc,
                ordinal: quota.issued,
                quota_maximum: quota.maximum,
            };
            certificate.validate()?;
            (
                Request::Issue(Box::new(certificate.clone())),
                Some(certificate),
                Some((quota, old.is_some())),
            )
        } else {
            if before.is_some()
                || !txn.family_is_empty(&f.certificates).map_err(bad)?
                || !txn.family_is_empty(&f.quotas).map_err(bad)?
            {
                return Err(Error::Integrity);
            }
            (Request::Initialize, None, None)
        };
        let after = ctx
            .prepare_owner(&self.owner, request)
            .map_err(bad)?
            .origin()
            .map_err(bad)?;
        if let Some(c) = &certificate {
            let expected = before.as_ref().ok_or(Error::Integrity)?.issue(c)?;
            if after != expected {
                return Err(Error::Integrity);
            }
            let lot = f
                .inventory
                .mint_origin(
                    txn,
                    c.origin_endpoint,
                    &c.commodity,
                    1,
                    c.grade,
                    c.source.clone(),
                    c.lot,
                )
                .map_err(bad)?;
            c.matches_lot(&lot)?;
            txn.insert(&f.certificates, &c.key(), c.clone())
                .map_err(bad)?;
            let (quota, exists) = quota.ok_or(Error::Integrity)?;
            let key = quota.key.key()?;
            if exists {
                txn.replace(&f.quotas, &key, quota).map_err(bad)?;
            } else {
                txn.insert(&f.quotas, &key, quota).map_err(bad)?;
            }
            txn.replace(&f.deployment, &(), after).map_err(bad)?;
        } else {
            txn.insert(&f.deployment, &(), after).map_err(bad)?;
        }
        Ok(AuthoritativeActionOutput::new(RulesValue::new(self.outcome.clone(),json!({"operation":operation,"lot":certificate.as_ref().map(|c|c.lot),"class":class})).map_err(bad)?)
   .with_definitions(vec![self.runtime.actions[if self.initialize{0}else{1}].clone(),self.outcome.clone(),self.runtime.config.policy.content.clone()]).with_eligible_witnesses(vec![inv.principal().clone()]))
    }
}
impl ActionProvider for Provider {
    fn validate_invocation(&self, inv: &ActionInvocation) -> Result<(), ActionError> {
        parse(inv, self.initialize)
            .map(|_| ())
            .map_err(|_| self.refusal.clone())
    }
    fn prepare_authoritative_with_context(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        self.prepare(inv, txn, ctx)
            .map_err(|_| self.refusal.clone())
    }
    fn project_offer(
        &self,
        action: &DefinitionRef,
        q: &mush_core::LegalActionQuery,
        engine: &mush_core::Engine,
    ) -> Result<Option<mush_core::ProviderOfferDraft>, ActionError> {
        let expected = &self.runtime.actions[if self.initialize { 0 } else { 1 }];
        if action != expected
            || q.truth_subject() != &(mush_core::SubjectKey::Object { id: q.actor() })
            || q.projected_subject() != &PerceivedRef::SelfActor
        {
            return Ok(None);
        }
        let state = engine.world.module_state::<CoreProductionState>();
        if state.is_none_or(|s| !s.ready) {
            return Ok(None);
        }
        if self.initialize {
            if q.actor() != mush_core::GOD
                || self.runtime.config.mode != CoreOriginMode::Initialize
                || state.is_some_and(|s| s.index.is_some())
            {
                return Ok(None);
            }
        } else if self.runtime.config.mode != CoreOriginMode::Active
            || state.is_none_or(|s| s.index.is_none())
            || authority::check(
                &engine.world,
                &self.runtime.definitions,
                &self.runtime.config.policy,
                q.actor(),
            )
            .is_err()
        {
            return Ok(None);
        }
        let fail = || self.refusal.clone();
        let defs = engine.registry.definitions().ok_or_else(fail)?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(mush_core::RevisionSet::default())
            .map_err(|_| fail())?;
        guards
            .add_definition(action.clone(), defs)
            .map_err(|_| fail())?;
        guards
            .add_definition(self.runtime.config.policy.content.clone(), defs)
            .map_err(|_| fail())?;
        let parameters = if self.initialize {
            vec![]
        } else {
            vec![mush_core::ProjectedParameter {
                id: CanonicalId::new("class").map_err(|_| fail())?,
                required: true,
                kind: mush_core::ProjectedParameterKind::Choice {
                    choices: DieClass::ALL
                        .into_iter()
                        .map(|class| {
                            serde_json::to_value(class)
                                .map_err(|_| fail())?
                                .as_str()
                                .map(str::to_owned)
                                .ok_or_else(fail)
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                },
            }]
        };
        Ok(Some(mush_core::ProviderOfferDraft {
            fixed_parameters: json!({}),
            parameters: mush_core::ProjectedParameterSchema::new(parameters).map_err(|_| fail())?,
            targets: mush_core::ProjectedTargetSchema::None,
            required_channels: vec![],
            expires: None,
            presentation: mush_core::ActionPresentation::new(
                if self.initialize {
                    "initialize-core-die-stock"
                } else {
                    "issue-core-die"
                },
                if self.initialize {
                    "Initialize certified die stock"
                } else {
                    "Issue a certified core die"
                },
                "Use the configured importer, stock vault and current civil-month quota.",
            )
            .map_err(|_| fail())?,
            guards: guards.finish().map_err(|_| fail())?,
        }))
    }
}

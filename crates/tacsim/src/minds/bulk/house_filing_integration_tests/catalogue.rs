//! Actual Market-owned observations of an acknowledged, measured house filing.
use super::*;
use mush_core::read_service::{OwnerReadError, OwnerReadHandle};
use mush_core::{ArgShape, CmdError, CommandSpec, Ctx, HookReadContext};
use mush_org::{
    AccessPolicy, CapabilityId, OrgAction, OrgId, OrgMutationOp, OrgMutationPlan, OrgScopeId,
    PolicyEffect, PolicyRule, PolicySubject, PrincipalId, ScopeSelector,
};
use mush_roll::commerce::{
    LicenceId,
    house_catalogue::{HouseIssuer, HouseSaleFacts, HouseSaleReader, HouseSaleRequest},
};
pub(super) type Answers = Arc<Mutex<Option<Result<HouseSaleFacts, OwnerReadError>>>>;
pub(super) struct Market {
    pub(super) shared: Shared,
    pub(super) answers: Answers,
}
struct Query {
    reader: OwnerReadHandle<HouseSaleReader>,
    answers: Answers,
}
impl mush_core::module::QueryHandler for Query {
    fn query(
        &self,
        _: &World,
        _: ObjId,
        _: &Value,
    ) -> Result<mush_proto::OutputEvent, mush_proto::RichText> {
        Err(mush_proto::RichText::plain("actual context required"))
    }
    fn query_with_context(
        &self,
        ctx: &HookReadContext<'_>,
        _: ObjId,
        args: &Value,
    ) -> Result<mush_proto::OutputEvent, mush_proto::RichText> {
        let refused = || mush_proto::RichText::plain("house fixture selector");
        let licence: LicenceId =
            serde_json::from_value(args.get("licence").cloned().ok_or_else(refused)?)
                .map_err(|_| refused())?;
        let req = if let Some(value) = args.get("issuer") {
            HouseSaleRequest::listed(
                licence,
                serde_json::from_value(value.clone()).map_err(|_| refused())?,
            )
        } else {
            HouseSaleRequest::publication(licence)
        };
        *self.answers.lock().map_err(|_| refused())? = Some(ctx.read_owner(&self.reader, req));
        Ok(mush_proto::OutputEvent::Sys(mush_proto::RichText::plain(
            "observed",
        )))
    }
}
impl GameModule for Market {
    fn name(&self) -> &'static str {
        "market"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        let manifest = self
            .shared
            .lock()
            .map_err(module_error)?
            .manifest
            .clone()
            .ok_or_else(|| ModError::msg("manifest"))?;
        let reader = HouseSaleReader::request(reg, &manifest)?;
        reg.query_handler(
            "house-catalogue.test",
            Arc::new(Query {
                reader,
                answers: self.answers.clone(),
            }),
        )?;
        // Test admin adapters call the actual Org service and persist its audit.
        reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
        reg.command(
            CommandSpec {
                name: "@house-policy-test".into(),
                shape: ArgShape::EqPair,
                ..CommandSpec::default()
            },
            Arc::new(admin),
        )
    }
}
fn admin(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.actor != GOD {
        return Err(CmdError::msg("fixture admin"));
    }
    let scope = OrgScopeId(ctx.args.lhs().parse().map_err(|_| CmdError::msg("scope"))?);
    let mode = ctx.args.rhs().unwrap_or_default().to_owned();
    let store = ctx
        .w
        .module_state_mut::<OrgStore>()
        .ok_or_else(|| CmdError::msg("Org"))?;
    let org = store
        .state()
        .scope(scope)
        .ok_or_else(|| CmdError::msg("scope"))?
        .org;
    let capability =
        CapabilityId::new("roll.mind.publish").ok_or_else(|| CmdError::msg("capability"))?;
    let ops = match mode.as_str() {
        "revoke" => store
            .state()
            .grants_in(org)
            .filter(|g| {
                g.grantee == GOD
                    && g.capability == capability
                    && g.scope == ScopeSelector::Exact(scope)
            })
            .map(|g| OrgMutationOp::RevokeGrant { grant: g.id })
            .collect(),
        "deny" => vec![OrgMutationOp::PutPolicy {
            policy: AccessPolicy {
                id: store.alloc_policy(),
                org,
                owner: PrincipalId::Organization(org),
                rules: vec![PolicyRule {
                    effect: PolicyEffect::Deny,
                    subject: PolicySubject::Principal(PrincipalId::Object(GOD)),
                    capability,
                    scope: ScopeSelector::Exact(scope),
                    conditions: vec![],
                }],
                default: PolicyEffect::Deny,
                revision: 1,
            },
        }],
        _ => return Err(CmdError::msg("mode")),
    };
    let plan = OrgMutationPlan::of(ops);
    mush_org::submit(
        ctx,
        &plan,
        OrgAction {
            action: "house-catalogue.test".into(),
            org: Some(org),
            detail: json!({"scope":scope,"mode":mode}),
        },
    )
    .map_err(|e| CmdError::msg(e.to_string()))
}
fn observe(
    f: &mut HouseFixture,
    licence: LicenceId,
    issuer: Option<HouseIssuer>,
) -> TestResult<Result<HouseSaleFacts, OwnerReadError>> {
    let disk = f.disk.clone();
    let state = f.engine.action_state().revision();
    let ic = f.engine.world.world_ic_now();
    let mut args = json!({"licence":licence});
    if let Some(issuer) = issuer {
        args["issuer"] = serde_json::to_value(issuer)?;
    }
    f.engine.step(
        mush_core::Inbound::Query {
            conn: 91,
            actor: GOD,
            id: 91,
            name: "house-catalogue.test".into(),
            args,
        },
        &mut Outbox::new(),
        &mut mush_core::Effects::new(),
    );
    let value = f
        .catalogue
        .lock()
        .map_err(module_error)?
        .take()
        .ok_or("query answer")?;
    check!(f.disk == disk, "read changed durable state");
    check_eq!(f.engine.action_state().revision(), state);
    check_eq!(f.engine.world.world_ic_now(), ic);
    Ok(value)
}
pub(super) fn exercise(
    registry: &Fixture,
    saved: &Durable,
    org: OrgId,
    scope: OrgScopeId,
    other: OrgScopeId,
) -> TestResult {
    let operation = one(rows(&saved.disk, "roll", "roll_mind_initial_filing"))?;
    let licence: LicenceId =
        serde_json::from_value(operation.get("licence").cloned().ok_or("licence")?)?;
    let issuer = HouseIssuer::new(org, scope)?;
    let mut f = HouseFixture::boot(registry, saved)?;
    let published = observe(&mut f, licence, None)??;
    let listed = observe(&mut f, licence, Some(issuer))??;
    check_eq!(published, listed);
    check_eq!(listed.issuer(), issuer);
    let fields = serde_json::to_value(&listed)?;
    check!(
        fields.get("source").is_none()
            && fields.get("envelope").is_none()
            && fields.get("executable").is_none()
    );
    check!(observe(&mut f, licence, Some(HouseIssuer::new(org, other)?))?.is_err());
    f.command(&format!("@house-policy-test {}=revoke", scope.0))?;
    check!(
        observe(&mut f, licence, None)?.is_err(),
        "former curator cannot publish by staff fallback"
    );
    check!(
        observe(&mut f, licence, Some(issuer))?.is_ok(),
        "house listing remains its own"
    );
    let departed = f.snapshot();
    let mut cold = HouseFixture::boot(registry, &departed)?;
    check!(observe(&mut cold, licence, None)?.is_err());
    check!(observe(&mut cold, licence, Some(issuer))?.is_ok());
    let mut denied = HouseFixture::boot(registry, saved)?;
    denied.command(&format!("@house-policy-test {}=deny", scope.0))?;
    check!(
        observe(&mut denied, licence, None)?.is_err(),
        "explicit scoped Deny wins over actual grant"
    );
    check!(observe(&mut denied, licence, Some(issuer))?.is_ok());
    denied.command("@org/status House=suspended")?;
    check!(observe(&mut denied, licence, Some(issuer))?.is_err());
    Ok(())
}

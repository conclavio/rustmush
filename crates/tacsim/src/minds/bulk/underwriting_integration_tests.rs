#![cfg(test)]

//! Actual certified paid mind, IC owners and registered coverage publication /
//! money / retention. Only the missing region transitions and submission-runtime
//! disposition are explicit private fixture boundaries, never owner receipts.
use super::*;
#[path = "underwriting_integration_tests/qualified.rs"]
pub(super) mod qualified;
use crate::underwriting::{
    CoverageCompletion, CoverageError, CoverageHost, CoverageProductV1, CoverageTermsV1,
};
use mush_core::{CmdError, Ctx, DefinitionDigest, IdempotencyKeyText};
use mush_economy::{AccountId as MoneyAccount, EconomyStore};
use mush_org::{OrgId, OrgScopeId, OrgStore};

const FIRST: u128 = 42_000_000;
type Call = Box<dyn for<'a> FnOnce(&mut Ctx<'a>) -> Result<(), CmdError> + Send>;
pub(super) type Calls = Arc<Mutex<Option<Call>>>;
pub(super) fn calls() -> Calls {
    Arc::new(Mutex::new(None))
}

/// Preserve every real Tacsim hook; the command merely supplies its actual
/// owner-stamped read context to the public host API, without changing state.
pub(super) struct Ingress {
    pub(super) inner: crate::TacsimRuleset,
    pub(super) calls: Calls,
}
impl GameModule for Ingress {
    fn name(&self) -> &'static str {
        "tacsim"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        self.inner.register(reg)?;
        let calls = self.calls.clone();
        reg.classify_commands(mush_core::CommandCompatibility::Authoritative);
        reg.command(
            mush_core::CommandSpec {
                name: "@coverage-host-fixture".into(),
                shape: mush_core::ArgShape::Raw,
                ..Default::default()
            },
            Arc::new(move |ctx: &mut Ctx<'_>| {
                let call = calls
                    .lock()
                    .map_err(|_| CmdError::msg("fixture lock"))?
                    .take()
                    .ok_or_else(|| CmdError::msg("fixture absent"))?;
                call(ctx)
            }),
        )?;
        Ok(())
    }
    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        data: ModuleBootData,
    ) -> Result<(), ModError> {
        self.inner.on_world_loaded(ctx, data)
    }
    fn on_all_loaded(&mut self, ctx: &mut ModuleCtx<'_>) -> Result<(), ModError> {
        self.inner.on_all_loaded(ctx)
    }
    fn finalize_owner_composition(&mut self, ctx: &mut ModuleCtx<'_>) -> Result<(), ModError> {
        self.inner.finalize_owner_composition(ctx)
    }
    fn boot_stream_requests(
        &self,
        kv: &[(smol_str::SmolStr, Value)],
    ) -> Result<Vec<(smol_str::SmolStr, u32)>, ModError> {
        self.inner.boot_stream_requests(kv)
    }
}
fn context<T: Send + 'static>(
    w: &mut IcWorld,
    actor: ObjId,
    f: impl for<'a> FnOnce(&mut Ctx<'a>) -> Result<T, CoverageError> + Send + 'static,
) -> TestResult<T> {
    let result = Arc::new(Mutex::new(None));
    let answer = result.clone();
    *w.coverage_calls.lock().map_err(module_error)? = Some(Box::new(move |ctx| {
        *answer.lock().map_err(|_| CmdError::msg("fixture answer"))? = Some(f(ctx));
        Ok(())
    }));
    w.engine.step(
        Inbound::Cmd {
            conn: if actor == BUYER { BUYER_CONN } else { CONN },
            actor,
            line: "@coverage-host-fixture".into(),
        },
        &mut Outbox::new(),
        &mut mush_core::Effects::new(),
    );
    let answer = result
        .lock()
        .map_err(module_error)?
        .take()
        .ok_or("actual owner ingress did not run")??;
    Ok(answer)
}
fn host(w: &IcWorld) -> TestResult<CoverageHost> {
    w.coverage_host
        .clone()
        .ok_or_else(|| "coverage host".into())
}
fn reference(w: &IcWorld, id: &str) -> TestResult<DefinitionRef> {
    w.engine
        .registry
        .definitions()
        .ok_or("definitions")?
        .iter()
        .map(|d| d.reference())
        .find(|r| {
            r.key().owner.as_str() == "tacsim"
                && r.key().kind.as_str() == "action"
                && r.key().id.as_str() == id
        })
        .cloned()
        .ok_or_else(|| "coverage action".into())
}
fn account(
    w: &IcWorld,
    owner: EconomicOwner,
    scope: Option<OrgScopeId>,
) -> TestResult<MoneyAccount> {
    let economy = w
        .engine
        .world
        .module_state::<EconomyStore>()
        .ok_or("economy")?;
    let snapshot = economy.snapshot();
    let mut rows = snapshot
        .accounts()
        .filter(|a| a.owner == owner && a.scope == scope && a.instrument.as_str() == "cr");
    let id = rows.next().ok_or("account")?.id;
    check!(rows.next().is_none(), "actual unique account");
    Ok(id)
}
fn money(w: &IcWorld, id: MoneyAccount) -> TestResult<Amount> {
    Ok(w.engine
        .world
        .module_state::<EconomyStore>()
        .ok_or("economy")?
        .snapshot()
        .account(id)
        .ok_or("account")?
        .posted_balance)
}
fn prepare(
    w: &mut IcWorld,
    actor: ObjId,
    action: &DefinitionRef,
    n: u128,
    params: Value,
) -> TestResult<(
    PreparedComposedTxn,
    PreparedAuthoritativeAction,
    ActionInvocation,
)> {
    prepare_key(w, actor, action, n, params, None)
}
fn prepare_key(
    w: &mut IcWorld,
    actor: ObjId,
    action: &DefinitionRef,
    n: u128,
    params: Value,
    key: Option<IdempotencyKeyText>,
) -> TestResult<(
    PreparedComposedTxn,
    PreparedAuthoritativeAction,
    ActionInvocation,
)> {
    let principal = PrincipalId::Object { id: actor };
    let query = w
        .engine
        .legal_action_query(principal.clone(), actor, PerceivedRef::SelfActor)?;
    let offers = w.engine.project_legal_actions(&query)?;
    let projected = offers
        .projected_for_action(action)
        .ok_or("actual signed coverage offer")?;
    let request = RequestUid::from_bytes(n.to_be_bytes());
    let resolved = w.engine.resolve_legal_action(
        &offers,
        mush_core::OfferExecutionContext {
            request,
            principal: principal.clone(),
            actor,
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal,
                    owner: action.key().owner.clone(),
                    namespace: CanonicalId::new("coverage_fixture")?,
                },
                1,
                key.map_or(IdempotencyKey::Request(request), IdempotencyKey::Caller),
            ),
            viewer_scope: query.viewer_scope(),
            now: None,
        },
        mush_core::OfferSubmission {
            offer: projected.offer(),
            subject: PerceivedRef::SelfActor,
            targets: vec![],
            parameters: params,
        },
    )?;
    let (exact, resolved, _) = resolved.into_authoritative_parts();
    check_eq!(&exact, action);
    let inv = resolved.invocation().clone();
    let (p, o) = w
        .engine
        .prepare_registered_composed_resolved_authoritative_action(action, &resolved, token(n))?;
    Ok((p, o, inv))
}
fn install(
    w: &mut IcWorld,
    action: &DefinitionRef,
    n: u128,
    prepared: (
        PreparedComposedTxn,
        PreparedAuthoritativeAction,
        ActionInvocation,
    ),
) -> TestResult {
    let (p, o, inv) = prepared;
    w.stage_action(action, n, p, o, &inv)?;
    let held = w
        .engine
        .take_staged_action()?
        .ok_or("actual coverage custody")?;
    let (bundle, installation) = held.into_parts();
    retain_writes(&mut w.disk, bundle.writes())?;
    w.engine.install_composed_action(installation)?;
    Ok(())
}
fn land_ticket(
    w: &mut IcWorld,
    t: crate::underwriting::CoverageTicket,
    n: u128,
) -> TestResult<CoverageCompletion> {
    let h = host(w)?;
    let actor = t.actor();
    let before = w.disk.clone();
    let prepared = prepare(w, actor, t.action(), n, json!({}))?;
    check!(
        w.disk == before,
        "coverage preparation changed durable data"
    );
    install(w, t.action(), n, prepared)?;
    context(w, actor, move |ctx| h.acknowledge(ctx, &t, token(n)))
}
fn observed_quote(
    w: &mut IcWorld,
    ticket: crate::underwriting::CoverageTicket,
    n: u128,
) -> TestResult<CoverageCompletion> {
    use crate::underwriting::CoverageSubmissionState as State;
    let h = host(w)?;
    // The caller installs the observed-submission runtime once per engine.
    let t = Arc::new(ticket);
    let (hc, tc) = (h.clone(), t.clone());
    check_eq!(
        context(w, BUYER, move |ctx| hc.submit(ctx, &tc))?,
        State::AwaitingAcknowledgement
    );
    let submitted = w
        .engine
        .take_observed_authoritative_submit()
        .ok_or("actual coverage submission")?;
    let (request, driver) = submitted.into_parts();
    check_eq!(request.action, *t.action());
    check_eq!(request.parameters, t.parameters());
    check_eq!(request.actor, BUYER);
    check!(request.targets.is_empty());
    let proof = *driver
        .ok_or("actual observation")?
        .downcast::<Arc<std::sync::atomic::AtomicBool>>()
        .map_err(|_| "observation type")?;
    for _ in 0..2 {
        let (hc, tc) = (h.clone(), t.clone());
        check_eq!(
            context(w, BUYER, move |ctx| hc.submit(ctx, &tc))?,
            State::AwaitingAcknowledgement
        );
        check!(w.engine.take_observed_authoritative_submit().is_none());
    }
    let (hc, tc) = (h.clone(), t.clone());
    check!(crate::test_checks::refused(context(
        w,
        BUYER,
        move |ctx| hc.acknowledge(ctx, &tc, token(n))
    ))?);
    let (hc, tc) = (h.clone(), t.clone());
    check!(crate::test_checks::refused(context(
        w,
        BUYER,
        move |ctx| hc.discard_unadmitted(ctx, &tc)
    ))?);
    // Only the submission-consumer disposition is modeled here. It cannot
    // manufacture a coverage row, native receipt, or successful ACK.
    proof.store(true, std::sync::atomic::Ordering::Release);
    check_eq!(h.submission_state(&t)?, State::Retryable);
    let (hc, tc) = (h.clone(), t.clone());
    let mut released = context(w, BUYER, move |ctx| hc.discard_unadmitted(ctx, &tc))?;
    check_eq!(released.ticket().parameters(), t.parameters());
    check!(h.submission_state(&t).is_err());
    let hc = h.clone();
    // The closure answers through the fixture's CoverageError, so its second
    // restore is checked here rather than inside it.
    let (restored, restored_again_refused) = context(w, BUYER, move |ctx| {
        let restored = hc.restore_unadmitted(ctx, &mut released)?;
        let again_refused = hc.restore_unadmitted(ctx, &mut released).is_err();
        Ok((restored, again_refused))
    })?;
    check!(
        restored_again_refused,
        "assertion failed: hc.restore_unadmitted(ctx, &mut released).is_err()"
    );
    check_eq!(restored.parameters(), request.parameters);
    check_eq!(restored.action(), t.action());
    let t = Arc::new(restored);
    let (hc, tc) = (h.clone(), t.clone());
    context(w, BUYER, move |ctx| hc.submit(ctx, &tc))?;
    let submitted = w
        .engine
        .take_observed_authoritative_submit()
        .ok_or("same coverage retry")?;
    let (retry, driver) = submitted.into_parts();
    check_eq!(retry.key, request.key);
    check_eq!(retry.parameters, request.parameters);
    check_eq!(retry.action, request.action);
    let proof = *driver
        .ok_or("retry observation")?
        .downcast::<Arc<std::sync::atomic::AtomicBool>>()
        .map_err(|_| "retry observation type")?;
    let prepared = prepare_key(w, BUYER, t.action(), n, json!({}), Some(retry.key))?;
    install(w, t.action(), n, prepared)?;
    // Contradictory definitive absence cannot defeat actual installed readback.
    proof.store(true, std::sync::atomic::Ordering::Release);
    let (hc, tc) = (h.clone(), t.clone());
    check!(crate::test_checks::refused(context(
        w,
        BUYER,
        move |ctx| hc.discard_unadmitted(ctx, &tc)
    ))?);
    let (hc, tc) = (h.clone(), t.clone());
    let acknowledged = context(w, BUYER, move |ctx| hc.acknowledge(ctx, &tc, token(n)))?;
    check!(h.submission_state(&t).is_err());
    Ok(acknowledged)
}
fn boot(
    f: &PurchaseFixture,
    base: &IcWorld,
    saved: &Durable,
    products: Vec<CoverageProductV1>,
) -> TestResult<IcWorld> {
    IcWorld::boot_source_qualified(
        f.into(),
        saved,
        base.base,
        base.field,
        base.hulls,
        true,
        base.pending_charters,
        base.paid_resales,
        base.registry_revocation.clone(),
        Some(products),
        false,
        base.transfer_facts.clone(),
        base.conduct,
        base.qualified_coverage.clone(),
    )
}
fn setup(
    f: &PurchaseFixture,
    base: &IcWorld,
    saved: &Durable,
) -> TestResult<(
    IcWorld,
    CoverageProductV1,
    MoneyAccount,
    MoneyAccount,
    OrgId,
    OrgScopeId,
)> {
    let mut w = base.reboot(f, saved)?;
    w.command("@org/create Coverage")?;
    w.command("@org/scope create Coverage=insurance")?;
    let state = w
        .engine
        .world
        .module_state::<OrgStore>()
        .ok_or("Org")?
        .state();
    let org = state
        .orgs()
        .find(|r| r.name.as_str() == "Coverage")
        .ok_or("insurer")?
        .id;
    let scope = state
        .scopes_in(org)
        .find(|r| r.name.as_str() == "insurance")
        .ok_or("scope")?
        .id;
    w.command(&format!(
        "@org/grant #{}=Coverage economy.coverage.publish scope={}",
        GOD.0, scope.0
    ))?;
    w.command(&format!("@account/open org:Coverage/{}=cr", scope.0))?;
    let treasury = account(&w, EconomicOwner::Organization(org), Some(scope))?;
    let payer = account(&w, EconomicOwner::Object(BUYER), None)?;
    let faucet = account(
        &w,
        EconomicOwner::System(
            mush_org::SystemPrincipalId::new("purchase.fixture").ok_or("faucet")?,
        ),
        None,
    )?;
    w.command(&format!("@econ/mint {faucet}={treasury} 20000"))?;
    w.command(&format!("@econ/mint {faucet}={payer} 2000"))?;
    let hull = mush_tactics::module::unit_of(&w.engine.world, w.hulls[0]).ok_or("hull")?;
    let product = CoverageProductV1::new(
        org,
        scope,
        treasury,
        GOD,
        CoverageTermsV1::launch(
            InstrumentId::new("cr").ok_or("cr")?,
            Amount::from_minor(100),
            Amount::from_minor(100_000),
            60,
        )?,
        vec![hull.schema().clone()],
        vec![selected_base(w.base, w.field)?.definition().clone()],
        vec![],
        600,
    )?;
    let configured = boot(f, &w, &w.snapshot(), vec![product.clone()])?;
    Ok((configured, product, payer, treasury, org, scope))
}

#[inline(never)]
fn publish_ordinary_product(
    f: &PurchaseFixture,
    w: &mut IcWorld,
    product: &CoverageProductV1,
) -> TestResult<Durable> {
    publication_fences(f, w, &w.snapshot(), product)?;
    let publication = reference(w, "publish_hull_coverage")?;
    check!(
        crate::test_checks::refused(prepare(
            w,
            BUYER,
            &publication,
            FIRST,
            json!({"product":product.digest()?.to_hex()})
        ))?,
        "customer is not publisher"
    );
    let p = prepare(
        w,
        GOD,
        &publication,
        FIRST,
        json!({"product":product.digest()?.to_hex()}),
    )?;
    install(w, &publication, FIRST, p)?;
    let published = w.snapshot();
    check_eq!(rows(&w.disk, "tacsim", "tacsim_coverage_products").len(), 1);
    check!(rows(&w.disk, "tacsim", "tacsim_coverage_policies").is_empty());
    Ok(published)
}

pub(super) fn exercise(f: &PurchaseFixture, base: &IcWorld, saved: &Durable) -> TestResult {
    eprintln!("M7_STORY phase=coverage_composed event=begin");
    let (mut w, product, payer, treasury, _, scope) = setup(f, base, saved)?;
    let published = publish_ordinary_product(f, &mut w, &product)?;
    let original_money = (money(&w, payer)?, money(&w, treasury)?);
    let h = host(&w)?;
    // A published product without a genuine acknowledged withheld departure
    // exposes neither quote nor Bind and creates no native money history.
    for id in ["quote_hull_coverage", "bind_hull_coverage"] {
        let action = reference(&w, id)?;
        check!(crate::test_checks::refused(prepare(
            &mut w,
            BUYER,
            &action,
            FIRST + 1,
            json!({})
        ))?);
    }
    check_eq!((money(&w, payer)?, money(&w, treasury)?), original_money);
    let hull = w.hulls[0];
    let zone = selected_base(w.base, w.field)?.definition().clone();
    let permit = ic_sortie::fixture_withheld(
        ic_sortie::fixture_start(&w.engine.world, BUYER, hull)?,
        zone.clone(),
        DefinitionDigest::from_bytes([71; 32]),
    )?;
    let pending = w
        .host
        .enqueue_withheld(&permit, LicenceId::new(2)?, vec![])?;
    check!(
        w.host
            .acknowledge_withheld(&w.engine.world, &pending)?
            .is_none()
    );
    w.land(pending.ticket(), FIRST + 2, hull, false)?;
    let departure = Arc::new(
        w.host
            .acknowledge_withheld(&w.engine.world, &pending)?
            .ok_or("actual withheld Open ACK")?,
    );
    let quote = h.quote(
        &w.engine.world,
        departure.clone(),
        token(FIRST),
        payer,
        Amount::from_minor(10_000),
    )?;
    w.engine.set_submission_runtime(Arc::new(Runtime))?;
    let quoted = observed_quote(&mut w, quote, FIRST + 3)?;
    check_eq!(quoted.kind(), "quoted");
    let value = one(rows(&w.disk, "tacsim", "tacsim_coverage_quotes"))?;
    check_eq!(value["risk"]["qualified"], Value::Null);
    check_eq!(value["terms"]["premium"], json!(100));
    check_eq!(value["terms"]["discount"], json!(0));
    bind_fences(f, &w, &w.snapshot(), departure.clone())?;
    let bind = h.bind(&w.engine.world, departure.clone(), token(FIRST + 3))?;
    let mut bound = land_ticket(&mut w, bind, FIRST + 4)?;
    check_eq!(bound.kind(), "bound");
    check_eq!(
        money(&w, payer)?,
        original_money
            .0
            .checked_sub(Amount::from_minor(100))
            .ok_or("premium")?
    );
    check_eq!(
        money(&w, treasury)?,
        original_money
            .1
            .checked_add(Amount::from_minor(100))
            .ok_or("premium")?
    );
    let policy = w
        .engine
        .world
        .module_state::<EconomyStore>()
        .ok_or("E4")?
        .coverage_history(token(FIRST + 4))?
        .clone();
    check_eq!(policy.consent().terms.premium, Amount::from_minor(100));
    let starts = policy.consent().terms.starts_at.clone();
    let ends = policy.consent().terms.ends_at.at;
    let installed = w.snapshot();
    cancel_branch(
        f,
        &w,
        &installed,
        permit.clone(),
        zone.clone(),
        payer,
        original_money.0,
    )?;
    loss_branch(
        f,
        &w,
        &installed,
        permit.clone(),
        zone.clone(),
        payer,
        treasury,
        original_money,
    )?;
    cold_corruption(f, &w, &installed)?;

    // A restarted host recovers only the same acknowledged departure and
    // installed policy. Neither the old nor the recovered ACK releases twice.
    let mut restarted = w.reboot(f, &installed)?;
    let recovered_host = host(&restarted)?;
    let retained_departure = departure.clone();
    let mut recovered = context(&mut restarted, BUYER, move |ctx| {
        recovered_host.recover_binding(ctx, retained_departure, token(FIRST + 4))
    })?;
    check_eq!(recovered.operation(), bound.operation());
    let h2 = h.clone();
    // As above: the repeat releases are checked outside the CoverageError closure.
    let (released, recovered_again_refused, bound_again_refused) =
        context(&mut w, BUYER, move |ctx| {
            let released = h2.release_departure(ctx, &mut recovered)?;
            let recovered_again = h2.release_departure(ctx, &mut recovered).is_err();
            let bound_again = h2.release_departure(ctx, &mut bound).is_err();
            Ok((released, recovered_again, bound_again))
        })?;
    check!(
        recovered_again_refused,
        "assertion failed: h2.release_departure(ctx, &mut recovered).is_err()"
    );
    check!(
        bound_again_refused,
        "assertion failed: h2.release_departure(ctx, &mut bound).is_err()"
    );
    check_eq!(released.open(), token(FIRST + 2));
    check_eq!(released.binding(), token(FIRST + 4));
    released.exact()?;
    check!(
        h.bind(&w.engine.world, departure, token(FIRST + 3))
            .is_err(),
        "released departure cannot buy twice"
    );
    // Expiry or an ordinary IC close cannot manufacture a coverage terminal.
    let h2 = h.clone();
    check!(crate::test_checks::refused(context(
        &mut w,
        GOD,
        move |ctx| h2.terminal(ctx, token(FIRST + 4), false)
    ))?);
    let mut ordinary_branch = w.reboot(f, &installed)?;
    let ordinary = ic_sortie::fixture_close(
        &ordinary_branch.engine.world,
        BUYER,
        token(FIRST + 2),
        ordinary_branch.base,
    )?;
    let ordinary = ordinary_branch.host.enqueue_close(&ordinary)?;
    check!(crate::test_checks::refused(
        ordinary_branch.prepare(&ordinary, FIRST + 5)
    )?);
    // Preserve the real insured Open and move beyond its cover term. A later
    // genuine clean close releases collateral while retaining the premium.
    w.engine.world.set_now(ends);
    check!(
        w.engine.world.world_ic_now().at >= ends
            && w.engine.world.world_ic_now().clock == starts.clock
    );
    let event = ic_sortie::fixture_covered_close(
        &w.engine.world,
        BUYER,
        token(FIRST + 2),
        w.base,
        permit,
        zone,
        DefinitionDigest::from_bytes([72; 32]),
    )?;
    let closing = w.host.enqueue_close(&event)?;
    w.land(&closing, FIRST + 6, hull, true)?;
    let h2 = h.clone();
    let terminal = context(&mut w, GOD, move |ctx| {
        h2.terminal(ctx, token(FIRST + 4), false)
    })?;
    let released = land_ticket(&mut w, terminal, FIRST + 7)?;
    check_eq!(released.kind(), "released");
    let final_history = w
        .engine
        .world
        .module_state::<EconomyStore>()
        .ok_or("E4")?
        .coverage_history(token(FIRST + 4))?;
    check!(final_history.terminal().is_some());
    check_eq!(
        money(&w, payer)?,
        original_money
            .0
            .checked_sub(Amount::from_minor(100))
            .ok_or("premium")?
    );
    let terminal_rows = rows(&w.disk, "tacsim", "tacsim_coverage_terminals");
    check_eq!(terminal_rows.len(), 1);
    check_eq!(terminal_rows[0]["policy"], json!(token(FIRST + 4)));
    check_eq!(terminal_rows[0]["operation"], json!(token(FIRST + 7)));
    cold_terminal_corruption(f, &w, &w.snapshot())?;
    let cold = w.reboot(f, &w.snapshot())?;
    check!(!cold.held(hull)?);
    check_eq!(
        cold.engine
            .world
            .module_state::<EconomyStore>()
            .ok_or("E4")?
            .coverage_history(token(FIRST + 4))?,
        final_history
    );
    // Actual absence and native/typed inverse are mandatory even with an empty
    // configured catalogue; historical publication remains unchanged.
    check!(crate::test_checks::refused(IcWorld::boot(
        f, &installed, w.base, w.field, w.hulls, true
    ))?);
    let _ = (published, scope);
    eprintln!("M7_STORY phase=coverage_composed event=complete");
    Ok(())
}

fn bind_fences(
    f: &PurchaseFixture,
    base: &IcWorld,
    saved: &Durable,
    departure: Arc<crate::underwriting::AcknowledgedDeparture>,
) -> TestResult {
    for phase in 0..3 {
        for failure in 0..3 {
            let mut w = base.reboot(f, saved)?;
            let h = host(&w)?;
            let ticket = h.bind(&w.engine.world, departure.clone(), token(FIRST + 3))?;
            let before = w.disk.clone();
            let invalidate = |w: &mut IcWorld| -> TestResult {
                if failure == 0 {
                    w.engine
                        .world
                        .get_mut(BUYER)
                        .ok_or("buyer")?
                        .flags
                        .insert(mush_core::Flags::GOING);
                    w.engine.world.take_dirty();
                } else {
                    w.engine.registry.freeze_mechanics_owner(
                        &OwnerId::new(if failure == 1 { "org" } else { "economy" })?,
                        mush_core::mechanics::IncidentId(421),
                        mush_core::mechanics::RecoveryReason::IntegrityViolation {
                            code: CanonicalId::new("coverage_fixture")?,
                        },
                    )?;
                }
                Ok(())
            };
            if phase == 0 {
                invalidate(&mut w)?;
                check!(crate::test_checks::refused(prepare(
                    &mut w,
                    BUYER,
                    ticket.action(),
                    FIRST + 50,
                    json!({})
                ))?);
            } else {
                let (prepared, output, invocation) =
                    prepare(&mut w, BUYER, ticket.action(), FIRST + 50, json!({}))?;
                if phase == 1 {
                    invalidate(&mut w)?;
                    check!(
                        w.stage_action(ticket.action(), FIRST + 50, prepared, output, &invocation)
                            .is_err()
                    );
                } else {
                    w.stage_action(ticket.action(), FIRST + 50, prepared, output, &invocation)?;
                    invalidate(&mut w)?;
                    check!(w.engine.take_staged_action().is_err());
                }
            }
            check!(w.disk == before, "failed Bind changed durable state");
        }
    }
    Ok(())
}
fn publication_fences(
    f: &PurchaseFixture,
    base: &IcWorld,
    saved: &Durable,
    product: &CoverageProductV1,
) -> TestResult {
    for phase in 0..3 {
        for frozen in [false, true] {
            let mut w = base.reboot(f, saved)?;
            let action = reference(&w, "publish_hull_coverage")?;
            let before = w.disk.clone();
            let invalidate = |w: &mut IcWorld| -> TestResult {
                if frozen {
                    w.engine.registry.freeze_mechanics_owner(
                        &OwnerId::new("org")?,
                        mush_core::mechanics::IncidentId(420),
                        mush_core::mechanics::RecoveryReason::IntegrityViolation {
                            code: CanonicalId::new("coverage_fixture")?,
                        },
                    )?;
                } else {
                    w.engine
                        .world
                        .get_mut(GOD)
                        .ok_or("publisher")?
                        .flags
                        .insert(mush_core::Flags::GOING);
                    let _ = w.engine.world.take_dirty();
                }
                Ok(())
            };
            if phase == 0 {
                invalidate(&mut w)?;
                check!(crate::test_checks::refused(prepare(
                    &mut w,
                    GOD,
                    &action,
                    FIRST + 20,
                    json!({"product":product.digest()?.to_hex()})
                ))?);
            } else {
                let (p, o, inv) = prepare(
                    &mut w,
                    GOD,
                    &action,
                    FIRST + 20,
                    json!({"product":product.digest()?.to_hex()}),
                )?;
                if phase == 1 {
                    invalidate(&mut w)?;
                    check!(w.stage_action(&action, FIRST + 20, p, o, &inv).is_err());
                } else {
                    w.stage_action(&action, FIRST + 20, p, o, &inv)?;
                    invalidate(&mut w)?;
                    check!(w.engine.take_staged_action().is_err());
                }
            }
            check!(
                w.disk == before,
                "publication refusal changed retained state"
            );
        }
    }
    Ok(())
}
fn cancel_branch(
    f: &PurchaseFixture,
    base: &IcWorld,
    saved: &Durable,
    permit: Arc<crate::underwriting::WithheldDeparture>,
    zone: DefinitionRef,
    payer: MoneyAccount,
    original: Amount,
) -> TestResult {
    let mut w = base.reboot(f, saved)?;
    let h = host(&w)?;
    let hull = w.hulls[0];
    let event = ic_sortie::fixture_covered_close(
        &w.engine.world,
        BUYER,
        token(FIRST + 2),
        w.base,
        permit,
        zone,
        DefinitionDigest::from_bytes([74; 32]),
    )?;
    let closing = w.host.enqueue_close(&event)?;
    w.land(&closing, FIRST + 40, hull, true)?;
    let hc = h.clone();
    let ticket = context(&mut w, BUYER, move |ctx| {
        hc.terminal(ctx, token(FIRST + 4), true)
    })?;
    check_eq!(ticket.actor(), BUYER);
    let done = land_ticket(&mut w, ticket, FIRST + 41)?;
    check_eq!(done.kind(), "released");
    check_eq!(
        money(&w, payer)?,
        original
            .checked_sub(Amount::from_minor(100))
            .ok_or("premium")?
    );
    let history = w
        .engine
        .world
        .module_state::<EconomyStore>()
        .ok_or("E4")?
        .coverage_history(token(FIRST + 4))?;
    check!(matches!(
        &history
            .terminal()
            .ok_or("actual cancellation")?
            .decision()
            .reason,
        mush_economy::managed_coverage::CoverageDecisionKindV1::Cancelled { .. }
    ));
    cold_terminal_corruption(f, &w, &w.snapshot())?;
    let cold = w.reboot(f, &w.snapshot())?;
    check_eq!(
        cold.engine
            .world
            .module_state::<EconomyStore>()
            .ok_or("E4")?
            .coverage_history(token(FIRST + 4))?,
        history
    );
    Ok(())
}
fn cold_terminal_corruption(f: &PurchaseFixture, base: &IcWorld, saved: &Durable) -> TestResult {
    for family in ["tacsim_coverage_terminals", "tacsim_coverage_receipts"] {
        let mut bad = saved.clone();
        let keys = bad
            .disk
            .iter()
            .filter_map(|(k, v)| {
                (v.get("family")
                    .and_then(|f| f.get("id"))
                    .and_then(Value::as_str)
                    == Some(family))
                .then_some(k.clone())
            })
            .collect::<Vec<_>>();
        check_eq!(keys.len(), 1, "actual terminal counterpart");
        for key in keys {
            bad.disk.remove(&key);
        }
        check!(
            crate::test_checks::refused(base.reboot(f, &bad))?,
            "missing exact terminal counterpart accepted"
        );
    }
    Ok(())
}
#[allow(clippy::too_many_arguments)]
fn loss_branch(
    f: &PurchaseFixture,
    base: &IcWorld,
    saved: &Durable,
    permit: Arc<crate::underwriting::WithheldDeparture>,
    zone: DefinitionRef,
    payer: MoneyAccount,
    treasury: MoneyAccount,
    original: (Amount, Amount),
) -> TestResult {
    let mut w = base.reboot(f, saved)?;
    let h = host(&w)?;
    let hull = w.hulls[0];
    let history = w
        .engine
        .world
        .module_state::<EconomyStore>()
        .ok_or("E4")?
        .coverage_history(token(FIRST + 4))?
        .clone();
    let loss = history.consent().terms.starts_at.at + 1;
    w.engine.world.set_now(loss);
    let event = ic_sortie::fixture_covered_loss(
        &w.engine.world,
        BUYER,
        token(FIRST + 2),
        w.base,
        permit,
        zone,
        DefinitionDigest::from_bytes([73; 32]),
    )?;
    let closing = w.host.enqueue_close(&event)?;
    w.land(&closing, FIRST + 30, hull, true)?;
    check!(crate::arena::damage::is_wreck(
        mush_tactics::module::unit_of(&w.engine.world, hull).ok_or("actual damaged hull")?
    ));
    let h2 = h.clone();
    check!(
        crate::test_checks::refused(context(&mut w, BUYER, move |ctx| h2.terminal(
            ctx,
            token(FIRST + 4),
            true
        )))?,
        "cancel cannot erase covered loss"
    );
    let h2 = h.clone();
    let pending = context(&mut w, GOD, move |ctx| {
        h2.terminal(ctx, token(FIRST + 4), false)
    })?;
    let before = w.disk.clone();
    check!(
        crate::test_checks::refused(prepare(
            &mut w,
            GOD,
            pending.action(),
            FIRST + 31,
            json!({})
        ))?,
        "actual delay not yet matured"
    );
    check!(w.disk == before);
    // Same pending intent, actual original-clock deadline, even after cover ends.
    let due = loss + history.consent().terms.claim_delay_seconds;
    check!(due > history.consent().terms.ends_at.at);
    w.engine.world.set_now(due);
    let paid = land_ticket(&mut w, pending, FIRST + 31)?;
    check_eq!(paid.kind(), "claimed");
    check_eq!(
        money(&w, payer)?,
        original
            .0
            .checked_sub(Amount::from_minor(100))
            .and_then(|a| a.checked_add(Amount::from_minor(8000)))
            .ok_or("claim arithmetic")?
    );
    check_eq!(
        money(&w, treasury)?,
        original
            .1
            .checked_add(Amount::from_minor(100))
            .and_then(|a| a.checked_sub(Amount::from_minor(8000)))
            .ok_or("claim arithmetic")?
    );
    let final_history = w
        .engine
        .world
        .module_state::<EconomyStore>()
        .ok_or("E4")?
        .coverage_history(token(FIRST + 4))?
        .clone();
    check!(matches!(
        &final_history
            .terminal()
            .ok_or("native claim")?
            .decision()
            .reason,
        mush_economy::managed_coverage::CoverageDecisionKindV1::Destroyed { .. }
    ));
    let cold = w.reboot(f, &w.snapshot())?;
    check_eq!(
        cold.engine
            .world
            .module_state::<EconomyStore>()
            .ok_or("E4")?
            .coverage_history(token(FIRST + 4))?,
        &final_history
    );
    check!(crate::arena::damage::is_wreck(
        mush_tactics::module::unit_of(&cold.engine.world, hull).ok_or("retained wreck")?
    ));
    Ok(())
}
fn cold_corruption(f: &PurchaseFixture, base: &IcWorld, saved: &Durable) -> TestResult {
    // Independently remove native publication, each typed coverage counterpart,
    // and permanent retention. No corrupted payload is printed on failure.
    for family in [
        "tacsim_coverage_products",
        "tacsim_coverage_quotes",
        "tacsim_coverage_policies",
        "tacsim_coverage_exposures",
        "tacsim_coverage_hulls",
        "roll_mind_coverage",
    ] {
        let mut bad = saved.clone();
        let keys = bad
            .disk
            .iter()
            .filter_map(|(k, v)| {
                (v.get("family")
                    .and_then(|f| f.get("id"))
                    .and_then(Value::as_str)
                    == Some(family))
                .then_some(k.clone())
            })
            .collect::<Vec<_>>();
        check!(!keys.is_empty(), "actual complete typed row selected");
        for key in keys {
            bad.disk.remove(&key);
        }
        check!(
            crate::test_checks::refused(base.reboot(f, &bad))?,
            "missing actual coverage counterpart accepted"
        );
    }
    let mut bad = saved.clone();
    let keys = bad
        .disk
        .keys()
        .filter(|(ns, k)| ns == "tacsim" && k.starts_with("coverage_publication:"))
        .cloned()
        .collect::<Vec<_>>();
    check_eq!(keys.len(), 1);
    for key in keys {
        bad.disk.remove(&key);
    }
    check!(
        crate::test_checks::refused(base.reboot(f, &bad))?,
        "missing native publication accepted"
    );
    Ok(())
}

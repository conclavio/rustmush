//! Only retained genuine host intents reach quote and monetary preparation.
//! Account, title and scoped insurer fences are repeated by actual native/Roll
//! owner patches; terminal cleanup never requires current source eligibility.
use super::{
    CoverageError, authority,
    families::{self, Families},
    host::{CoverageHost, Intent},
    policy::{CloseProof, Exposure, HullHead, Policy, Receipt, Terminal},
    protocol::Protocol,
    records::{Publication, Quote, Risk},
};
use mush_core::action::{
    ActionError, ActionInvocation, ActionProvider, AuthoritativeActionOutput, PerceivedRef,
    PrincipalId,
};
use mush_core::participant::ParticipantHandle;
use mush_core::{ActionTxn, DefinitionDigest, DefinitionRef, PrepareCtx, RulesValue};
use mush_economy::managed_coverage::{
    CoverageBindRequest, CoverageDecisionKindV1, CoverageDecisionV1, CoverageTerminalRequest,
};
use mush_economy::registry_filing::{EconomyRequest, InitialFilingParticipant};
use mush_economy::{AccountStatus, EconomyStore, InstrumentStatus};
use mush_roll::commerce::coverage::{PreparedCoverageOpen, RegisteredCoverageWriter};
use serde::Deserialize;
fn bad(_: impl std::fmt::Display) -> CoverageError {
    CoverageError::Authority
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Parameters {
    ticket: DefinitionDigest,
}
pub(super) struct Provider {
    pub(super) protocol: Protocol,
    pub(super) action: DefinitionRef,
    pub(super) outcome: DefinitionRef,
    pub(super) index: usize,
    pub(super) families: Families,
    pub(super) host: CoverageHost,
    pub(super) writer: RegisteredCoverageWriter,
    pub(super) economy: Option<ParticipantHandle<InitialFilingParticipant>>,
    pub(super) refusal: ActionError,
    pub(super) conduct_reader: Option<
        mush_core::read_service::OwnerReadHandle<mush_roll::commerce::conduct::ConductReader>,
    >,
}
impl Provider {
    fn intent(&self, inv: &ActionInvocation) -> Result<(DefinitionDigest, Intent), CoverageError> {
        if inv.principal() != &(PrincipalId::Object { id: inv.actor() })
            || inv.subject() != &PerceivedRef::SelfActor
            || !inv.targets().is_empty()
            || inv.idempotency().scope().owner.as_str() != crate::NS
        {
            return Err(CoverageError::Authority);
        }
        let p: Parameters = serde_json::from_value(inv.parameters().clone()).map_err(bad)?;
        Ok((
            p.ticket,
            self.host.lookup(p.ticket, inv.actor(), self.index)?,
        ))
    }
    fn open(
        &self,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        departure: &crate::minds::ic_sortie::AcknowledgedDeparture,
    ) -> Result<PreparedCoverageOpen, CoverageError> {
        departure.recheck(ctx.world().map_err(bad)?).map_err(bad)?;
        let observed = self
            .writer
            .observe(ctx, txn, departure.open())
            .map_err(bad)?;
        let f = observed.facts();
        if f.actor() != departure.actor()
            || f.unit() != departure.unit()
            || f.hull_revision() != departure.unit_revision()
            || f.hull_schema() != departure.unit_schema()
            || f.seal_digest() != departure.seal_digest()
            || f.admission_digest().map_err(bad)? != departure.admission_digest()
        {
            return Err(CoverageError::Authority);
        }
        Ok(observed)
    }
    fn product(
        &self,
        ctx: &PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        id: mush_core::CommitToken,
    ) -> Result<Publication, CoverageError> {
        let p = families::read(txn, &self.families.products, id.to_hex())?;
        // The native immutable mirror closes publication provenance without
        // treating its decoded historical authorization as a current grant.
        let w = ctx.world().map_err(bad)?;
        if w.module_state::<super::metadata::Publications>()
            .and_then(|s| s.rows.get(&id))
            .is_none_or(|actual| actual.as_ref() != &p)
            || p.protocol != self.protocol
        {
            return Err(CoverageError::Authority);
        }
        let reg = self.host.ready(w, self.index)?;
        reg.readiness.issuer(w, &p.product)?;
        reg.readiness
            .references(p.product.hulls().iter().chain(p.product.zones()))?;
        if p.product.adjudicator() != self.host.service {
            return Err(CoverageError::Authority);
        }
        Ok(p)
    }
    fn prepare(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, CoverageError> {
        let (request, intent) = self.intent(inv)?;
        self.host.ready(ctx.world().map_err(bad)?, self.index)?;
        authority::player(ctx.world().map_err(bad)?, inv.actor())?;
        let operation = ctx.commit_token();
        let retained_intent = intent.clone();
        let kind = match intent {
            Intent::Quote {
                departure,
                publication,
                payer,
                value,
            } if self.index == 1 => {
                let observed = self.open(ctx, txn, &departure)?;
                let p = self.product(ctx, txn, publication)?;
                let product = &p.product;
                let qualified = super::qualification::current(
                    ctx,
                    txn,
                    self.conduct_reader.as_ref(),
                    &p,
                    &departure,
                    observed.facts(),
                )?;
                let illustration = product.illustrate(value)?;
                let amounts = if qualified.is_some() {
                    illustration.with_qualified_safe_conduct()
                } else {
                    illustration.ordinary()
                };
                let w = ctx.world().map_err(bad)?;
                let now = w.now();
                if now < p.posted_at_utc
                    || !product.hulls().contains(departure.unit_schema())
                    || !product.zones().contains(departure.zone())
                {
                    return Err(CoverageError::Authority);
                }
                accounts(w, inv.actor(), payer, product)?;
                let q = Quote {
                    format: 1,
                    request,
                    operation,
                    action: self.action.clone(),
                    protocol: self.protocol.clone(),
                    publication,
                    publication_digest: p.digest()?,
                    product_digest: product.digest()?,
                    risk: Risk {
                        ic_open: departure.open(),
                        seal_digest: departure.seal_digest(),
                        admission_digest: departure.admission_digest(),
                        departure_digest: departure.digest(),
                        hull: departure.unit(),
                        hull_title_revision: observed.facts().hull_title_revision(),
                        hull_revision: departure.unit_revision(),
                        hull_schema: departure.unit_schema().clone(),
                        zone: departure.zone().clone(),
                        qualified: qualified.as_ref().map(|(selection, _)| selection.clone()),
                    },
                    terms: mush_economy::managed_coverage::CoverageQuoteTermsV1 {
                        format: 1,
                        buyer: inv.actor(),
                        insurer: product.insurer(),
                        scope: product.scope(),
                        treasury: product.treasury(),
                        payer,
                        instrument: product.terms().instrument().clone(),
                        adjudicator: product.adjudicator(),
                        insured_value: value,
                        base_premium: amounts.base_premium(),
                        discount: amounts.discount(),
                        premium: amounts.premium(),
                        liability: amounts.maximum_payout(),
                        duration_seconds: product.terms().term_seconds(),
                        claim_delay_seconds: product.terms().claim_delay_seconds(),
                        quoted_at_utc: now,
                        expires_at_utc: now
                            .checked_add(product.quote_seconds())
                            .ok_or(CoverageError::Arithmetic)?,
                    },
                };
                q.matches_publication(&p)?;
                self.families.admit(txn, 1)?;
                families::insert(txn, &self.families.quotes, &q)?;
                let check = super::fence::HostCheck::capture(
                    self.host.clone(),
                    self.action.clone(),
                    operation,
                    inv.actor(),
                    self.index,
                    request,
                    retained_intent,
                    Some((product.clone(), payer)),
                    qualified.map(|(_, proof)| proof),
                );
                self.writer.quote(ctx, observed, check).map_err(bad)?;
                "quoted"
            }
            Intent::Bind { departure, quote } if self.index == 2 => {
                let observed = self.open(ctx, txn, &departure)?;
                let q = families::read(txn, &self.families.quotes, quote.to_hex())?;
                let p = self.product(ctx, txn, q.publication)?;
                q.matches_publication(&p)?;
                if q.risk.ic_open != departure.open()
                    || q.risk.departure_digest != departure.digest()
                    || q.terms.buyer != inv.actor()
                    || q.risk.hull_title_revision != observed.facts().hull_title_revision()
                {
                    return Err(CoverageError::Authority);
                }
                let conduct = if let Some(expected) = &q.risk.qualified {
                    let (selection, proof) = super::qualification::current(
                        ctx,
                        txn,
                        self.conduct_reader.as_ref(),
                        &p,
                        &departure,
                        observed.facts(),
                    )?
                    .ok_or(CoverageError::Authority)?;
                    if &selection != expected {
                        return Err(CoverageError::Authority);
                    }
                    Some(proof)
                } else {
                    None
                };
                let hull_key = format!("{:08x}", q.risk.hull.0);
                let head = txn
                    .get(&self.families.hulls, &hull_key)
                    .map_err(bad)?
                    .cloned();
                let (sequence, previous) = if let Some(h) = &head {
                    if h.active.is_some() {
                        return Err(CoverageError::Authority);
                    }
                    let old = families::read(txn, &self.families.policies, h.latest.to_hex())?;
                    if *h != HullHead::from_policy(&old)? {
                        return Err(CoverageError::Product);
                    }
                    let d = old
                        .history
                        .terminal()
                        .ok_or(CoverageError::Authority)?
                        .decision();
                    let w = ctx.world().map_err(bad)?;
                    let now = w.world_ic_now();
                    if w.now() < d.posted_at_utc
                        || now.clock == d.at_ic.clock && now.at < d.at_ic.at
                    {
                        return Err(CoverageError::Clock);
                    }
                    (
                        h.sequence.checked_add(1).ok_or(CoverageError::Bound)?,
                        Some(h.latest),
                    )
                } else {
                    (1, None)
                };
                let output = ctx
                    .prepare_owner(
                        self.economy.as_ref().ok_or(CoverageError::Authority)?,
                        EconomyRequest::CoverageBind(CoverageBindRequest {
                            basis: q.native_basis()?,
                            terms: q.terms.clone(),
                        }),
                    )
                    .map_err(bad)?
                    .coverage()
                    .map_err(bad)?;
                let check = super::fence::HostCheck::capture(
                    self.host.clone(),
                    self.action.clone(),
                    operation,
                    inv.actor(),
                    self.index,
                    request,
                    retained_intent,
                    Some((p.product.clone(), q.terms.payer)),
                    conduct,
                );
                let actual = self
                    .writer
                    .bind(ctx, txn, observed, output, check)
                    .map_err(bad)?;
                let policy = Policy {
                    format: 1,
                    request,
                    operation,
                    action: self.action.clone(),
                    protocol: self.protocol.clone(),
                    sequence,
                    previous,
                    quote,
                    quote_digest: q.digest()?,
                    history: actual.history().clone(),
                };
                policy.matches_quote(&q)?;
                super::capacity::before_money(&self.families, &policy)?;
                actual
                    .retention()
                    .matches_history(&policy.history)
                    .map_err(bad)?;
                self.families.admit(txn, 2)?;
                families::insert(txn, &self.families.policies, &policy)?;
                families::insert(
                    txn,
                    &self.families.exposures,
                    &Exposure::from_policy(&policy)?,
                )?;
                let after = HullHead::from_policy(&policy)?;
                if head.is_some() {
                    families::replace(txn, &self.families.hulls, &after)?;
                } else {
                    families::insert(txn, &self.families.hulls, &after)?;
                }
                ctx.reserve_hull_coverage_completion(txn).map_err(bad)?;
                "bound"
            }
            Intent::Terminal {
                close,
                policy,
                cancel,
            } if self.index == 3 || self.index == 4 => {
                let before = families::read(txn, &self.families.policies, policy.to_hex())?;
                if before.history.terminal().is_some() {
                    return Err(CoverageError::Authority);
                }
                let c = before.history.consent();
                let proof = CloseProof::capture(ctx.world().map_err(bad)?, &close)?;
                if proof.open != c.basis.ic_open {
                    return Err(CoverageError::Authority);
                }
                let pays = proof.covered_loss(c)?;
                if self.index != if pays { 3 } else { 4 }
                    || cancel && pays
                    || inv.actor()
                        != if cancel {
                            c.terms.buyer
                        } else {
                            self.host.service
                        }
                {
                    return Err(CoverageError::Authority);
                }
                let reason = if pays {
                    CoverageDecisionKindV1::Destroyed {
                        loss: proof.close,
                        loss_digest: proof.digest,
                        occurred_at: proof.at_ic.clone(),
                        zone: proof.zone.clone(),
                        transition: proof.transition,
                    }
                } else if cancel {
                    CoverageDecisionKindV1::Cancelled {
                        close: proof.close,
                        close_digest: proof.digest,
                        transition: proof.transition,
                    }
                } else {
                    let now = ctx.world().map_err(bad)?.world_ic_now();
                    if now.clock == c.terms.ends_at.clock && now.at >= c.terms.ends_at.at {
                        CoverageDecisionKindV1::ExpiredClean {
                            close: proof.close,
                            close_digest: proof.digest,
                            transition: proof.transition,
                        }
                    } else {
                        CoverageDecisionKindV1::CleanClose {
                            close: proof.close,
                            close_digest: proof.digest,
                            transition: proof.transition,
                        }
                    }
                };
                let w = ctx.world().map_err(bad)?;
                let decision = CoverageDecisionV1 {
                    format: 1,
                    operation,
                    action: self.action.clone(),
                    actor: inv.actor(),
                    at_ic: w.world_ic_now(),
                    posted_at_utc: w.now(),
                    reason,
                };
                decision.matches_consent(c).map_err(bad)?;
                let h =
                    families::read(txn, &self.families.hulls, format!("{:08x}", c.basis.hull.0))?;
                if h != HullHead::from_policy(&before)? || h.active != Some(policy) {
                    return Err(CoverageError::Product);
                }
                let output = ctx
                    .prepare_owner(
                        self.economy.as_ref().ok_or(CoverageError::Authority)?,
                        EconomyRequest::CoverageTerminal(CoverageTerminalRequest {
                            binding: policy,
                            decision,
                        }),
                    )
                    .map_err(bad)?
                    .coverage()
                    .map_err(bad)?;
                let check = super::fence::HostCheck::capture(
                    self.host.clone(),
                    self.action.clone(),
                    operation,
                    inv.actor(),
                    self.index,
                    request,
                    retained_intent,
                    None,
                    None,
                );
                let actual = self.writer.finish(ctx, txn, output, check).map_err(bad)?;
                let mut after = before.clone();
                after.history = actual.history().clone();
                after.terminal_of(&before)?;
                let terminal = Terminal {
                    format: 1,
                    request,
                    operation,
                    action: self.action.clone(),
                    protocol: self.protocol.clone(),
                    policy,
                    policy_digest: after.digest()?,
                    native_history: after.history.digest().map_err(bad)?,
                    close: proof,
                };
                terminal.matches_policy(&after)?;
                families::replace(txn, &self.families.policies, &after)?;
                families::replace(
                    txn,
                    &self.families.exposures,
                    &Exposure::from_policy(&after)?,
                )?;
                families::replace(txn, &self.families.hulls, &HullHead::from_policy(&after)?)?;
                families::insert(txn, &self.families.terminals, &terminal)?;
                families::insert(txn, &self.families.receipts, &Receipt::from_policy(&after)?)?;
                ctx.finish_hull_coverage_completion(txn, policy)
                    .map_err(bad)?;
                if pays { "claimed" } else { "released" }
            }
            _ => return Err(CoverageError::Authority),
        };
        let result = RulesValue::new(
            self.outcome.clone(),
            serde_json::json!({"operation":operation,"kind":kind}),
        )
        .map_err(bad)?;
        Ok(AuthoritativeActionOutput::new(result)
            .with_definitions(vec![
                self.action.clone(),
                self.outcome.clone(),
                self.protocol.manifest.clone(),
            ])
            .with_eligible_witnesses(vec![inv.principal().clone()]))
    }
}
impl ActionProvider for Provider {
    fn validate_invocation(&self, inv: &ActionInvocation) -> Result<(), ActionError> {
        self.intent(inv)
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
        e: &mush_core::Engine,
    ) -> Result<Option<mush_core::ProviderOfferDraft>, ActionError> {
        if action != &self.action
            || q.truth_subject() != &(mush_core::SubjectKey::Object { id: q.actor() })
            || q.projected_subject() != &PerceivedRef::SelfActor
            || self.host.ready(&e.world, self.index).is_err()
        {
            return Ok(None);
        }
        let Some(request) = self
            .host
            .selected(q.actor(), self.index)
            .map_err(|_| self.refusal.clone())?
        else {
            return Ok(None);
        };
        let fail = || self.refusal.clone();
        let definitions = e.registry.definitions().ok_or_else(fail)?;
        let mut guards = e
            .registry
            .server_guard_set_builder(mush_core::RevisionSet::default())
            .map_err(|_| fail())?;
        for r in [&self.action, &self.outcome, &self.protocol.manifest] {
            guards
                .add_definition(r.clone(), definitions)
                .map_err(|_| fail())?;
        }
        Ok(Some(mush_core::ProviderOfferDraft {
            fixed_parameters: serde_json::json!({"ticket":request}),
            parameters: mush_core::ProjectedParameterSchema::new(vec![]).map_err(|_| fail())?,
            targets: mush_core::ProjectedTargetSchema::None,
            required_channels: vec![],
            expires: None,
            presentation: mush_core::ActionPresentation::new(
                "hull-coverage",
                "Apply hull coverage transition",
                "Apply the exact transition retained by your coverage host.",
            )
            .map_err(|_| fail())?,
            guards: guards.finish().map_err(|_| fail())?,
        }))
    }
}
pub(super) fn accounts(
    world: &mush_core::World,
    actor: mush_core::ObjId,
    payer: mush_economy::AccountId,
    p: &super::CoverageProductV1,
) -> Result<(), CoverageError> {
    let e = world
        .module_state::<EconomyStore>()
        .ok_or(CoverageError::Authority)?;
    if e.is_frozen()
        || world
            .module_state_owner(std::any::TypeId::of::<EconomyStore>())
            .is_none_or(|o| o.as_str() != "economy")
    {
        return Err(CoverageError::Authority);
    }
    let snap = e.snapshot();
    let instrument = p.terms().instrument();
    if snap
        .instrument(instrument)
        .is_none_or(|i| i.status != InstrumentStatus::Active || !i.transferable)
        || snap.account(payer).is_none_or(|a| {
            a.owner != mush_org::PrincipalId::Object(actor)
                || a.scope.is_some()
                || a.status != AccountStatus::Active
                || &a.instrument != instrument
        })
        || snap.account(p.treasury()).is_none_or(|a| {
            a.owner != mush_org::PrincipalId::Organization(p.insurer())
                || a.scope != Some(p.scope())
                || a.status != AccountStatus::Active
                || &a.instrument != instrument
        })
        || payer == p.treasury()
    {
        return Err(CoverageError::Authority);
    }
    Ok(())
}

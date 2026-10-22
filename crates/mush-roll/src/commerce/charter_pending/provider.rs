//! Actual seller consent, buyer hold and private terminal providers.
use super::{
    PendingCharterTransferConfig, Readiness, bad, current, history, protocol,
    publication::{Admission, Mode, PendingFence, PendingRoots},
    records::*,
    service,
};
use crate::commerce::{
    CharterId,
    charter_transfer::{
        economic::{PendingEconomic, TransferEconomic},
        records::{CharterOwnershipHead, CharterTransfer},
    },
    families::CommerceFamilies,
    participant::{RootParticipant, RootRequest},
    records::{PublicationError, Record},
    roots::{RequiredRoot, RootAnchor},
};
use mush_core::action::{
    ActionError, ActionErrorCode, ActionInvocation, ActionProvider, AuthoritativeActionOutput,
    PerceivedRef, PrincipalId,
};
use mush_core::mechanics::RulesValue;
use mush_core::participant::ParticipantHandle;
use mush_core::{ActionTxn, CommitToken, DefinitionRef, ModError, ObjId, PrepareCtx, Registrar};
use mush_economy::{
    AccountId, Amount,
    managed_charter_transfer::{
        CharterPendingDecisionKindV1 as Reason, CharterPendingDecisionV1,
        CharterPendingFundingRequest, CharterPendingTerminalRequest,
    },
    registry_filing::{EconomyRequest, InitialFilingParticipant},
};
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OfferWire {
    charter: String,
    buyer: String,
    seller_account: String,
    price_minor: String,
    valid_for_seconds: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FundWire {
    offer: CommitToken,
    payer: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TerminalWire {
    funding: CommitToken,
}
fn decimal<T: std::str::FromStr + ToString>(value: &str) -> Result<T, PublicationError> {
    let n = value.parse::<T>().map_err(bad)?;
    if n.to_string() != value {
        return Err(PublicationError::Invalid);
    }
    Ok(n)
}
struct Provider {
    phase: usize,
    action: DefinitionRef,
    outcome: DefinitionRef,
    retention: DefinitionRef,
    f: CommerceFamilies,
    roots: ParticipantHandle<RootParticipant>,
    economy: Option<ParticipantHandle<InitialFilingParticipant>>,
    ready: Readiness,
    runtime: service::Runtime,
    config: PendingCharterTransferConfig,
    refusal: ActionError,
}
impl Provider {
    fn parse(&self, inv: &ActionInvocation) -> Result<(), PublicationError> {
        if inv.principal() != &(PrincipalId::Object { id: inv.actor() })
            || inv.subject() != &PerceivedRef::SelfActor
            || !inv.targets().is_empty()
            || inv.idempotency().scope().owner != self.action.key().owner
        {
            return Err(PublicationError::Invalid);
        }
        match self.phase {
            0 => {
                let _: OfferWire = serde_json::from_value(inv.parameters().clone()).map_err(bad)?;
            }
            1 => {
                let _: FundWire = serde_json::from_value(inv.parameters().clone()).map_err(bad)?;
            }
            2 | 3 => {
                let w: TerminalWire =
                    serde_json::from_value(inv.parameters().clone()).map_err(bad)?;
                let intent = self.runtime.intent(inv, &self.action)?;
                if w.funding != intent.funding || intent.actor != self.config.service_actor {
                    return Err(PublicationError::Mismatch);
                }
            }
            _ => return Err(PublicationError::Invalid),
        }
        Ok(())
    }
    fn admission(
        &self,
        ctx: &PrepareCtx<'_>,
        charter: CharterId,
        seller: ObjId,
        buyer: ObjId,
        payout: AccountId,
        deadline: i64,
        claims: Option<ClaimObservation>,
    ) -> Result<Admission, PublicationError> {
        self.ready.check()?;
        let w = ctx.world().map_err(bad)?;
        let e = w
            .module_state::<mush_economy::EconomyStore>()
            .ok_or(PublicationError::Unavailable)?;
        Ok(Admission {
            charter,
            seller,
            buyer,
            title: e
                .snapshot()
                .asset(charter.object())
                .ok_or(PublicationError::Missing)?
                .clone(),
            payout: e
                .snapshot()
                .account(payout)
                .ok_or(PublicationError::Missing)?
                .clone(),
            deadline,
            claims,
        })
    }
    fn fence(
        &self,
        inv: &ActionInvocation,
        ctx: &PrepareCtx<'_>,
        mode: Mode,
    ) -> Result<PendingFence, PublicationError> {
        let world = ctx.world().map_err(bad)?;
        let fence = PendingFence {
            action: self.action.clone(),
            commit: ctx.commit_token(),
            actor: inv.actor(),
            parameters: inv.parameters().clone(),
            at_ic: world.world_ic_now(),
            at_utc: world.now(),
            ready: self.ready.clone(),
            mode,
        };
        fence.check_world(world)?;
        Ok(fence)
    }
    fn roots(
        &self,
        ctx: &mut PrepareCtx<'_>,
        old: Vec<RequiredRoot>,
        added: Vec<RequiredRoot>,
        fence: PendingFence,
    ) -> Result<(), PublicationError> {
        ctx.prepare_owner(
            &self.roots,
            RootRequest::CharterPending(Box::new(PendingRoots {
                commit: ctx.commit_token(),
                old,
                added,
                fence,
            })),
        )
        .map_err(bad)?;
        Ok(())
    }
    fn offer(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<PendingOffer, PublicationError> {
        let w: OfferWire = serde_json::from_value(inv.parameters().clone()).map_err(bad)?;
        let charter = CharterId::new(decimal(&w.charter)?).map_err(bad)?;
        let buyer = ObjId(decimal(&w.buyer)?);
        let account = AccountId(decimal(&w.seller_account)?);
        let price = Amount::from_minor(decimal(&w.price_minor)?);
        let duration: i64 = decimal(&w.valid_for_seconds)?;
        if !(1..=crate::commerce::charter_transfer::records::MAX_OFFER_SECONDS).contains(&duration)
            || price.minor() <= 0
        {
            return Err(PublicationError::Invalid);
        }
        let pair = ctx.distinct_personal_accounts(buyer).map_err(bad)?;
        if pair.requester() != inv.actor() || pair.other() != buyer {
            return Err(PublicationError::Mismatch);
        }
        let world = ctx.world().map_err(bad)?;
        current::require_clear_in_txn(txn, world, &self.f, charter)?;
        let basis = current::basis(txn, world, &self.f, charter, inv.actor())?;
        let deadline = world
            .now()
            .checked_add(duration)
            .ok_or(PublicationError::Capacity)?;
        let admission =
            self.admission(ctx, charter, inv.actor(), buyer, account, deadline, None)?;
        let offer = PendingOffer {
            format: 1,
            operation: ctx.commit_token(),
            action: self.action.clone(),
            charter,
            birth: basis.birth.commit,
            ownership: basis.ownership,
            seller: inv.actor(),
            buyer,
            seller_account: account,
            instrument: admission.payout.instrument.clone(),
            price,
            title_revision: admission.title.revision,
            line: basis.birth.line,
            filed: basis.filed.content.id(),
            revision: basis.filed.content.revision(),
            owner_term: PendingOwnerTerm::capture(&basis.term)?,
            offered_at_ic: world.world_ic_now(),
            offered_at_utc: world.now(),
            expires_at_utc: deadline,
            settle_seconds: self.config.settle_seconds,
        };
        offer.matches_birth(&basis.birth)?;
        let fence = self.fence(inv, ctx, Mode::Admission(Box::new(admission)))?;
        self.roots(
            ctx,
            vec![RequiredRoot {
                anchor: RootAnchor::Filed(offer.filed),
                identity: offer.revision,
            }],
            vec![history::offer_root(&offer)?],
            fence,
        )?;
        txn.insert(&self.f.charter_pending_offers, &offer.key(), offer.clone())
            .map_err(bad)?;
        if txn
            .get(&self.f.charter_pending_offers, &offer.key())
            .map_err(bad)?
            != Some(&offer)
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(offer)
    }
    fn fund(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<PendingOffer, PublicationError> {
        let w: FundWire = serde_json::from_value(inv.parameters().clone()).map_err(bad)?;
        let payer = AccountId(decimal(&w.payer)?);
        let offer = txn
            .get(&self.f.charter_pending_offers, &w.offer.to_hex())
            .map_err(bad)?
            .cloned()
            .ok_or(PublicationError::Missing)?;
        offer.validate()?;
        if offer.buyer != inv.actor() {
            return Err(PublicationError::Mismatch);
        }
        let pair = ctx.distinct_personal_accounts(offer.seller).map_err(bad)?;
        if pair.requester() != inv.actor() || pair.other() != offer.seller {
            return Err(PublicationError::Mismatch);
        }
        let world = ctx.world().map_err(bad)?;
        current::require_clear_in_txn(txn, world, &self.f, offer.charter)?;
        let basis = current::basis(txn, world, &self.f, offer.charter, offer.seller)?;
        current::matches_offer(&basis, &offer)?;
        crate::commerce::registry_revocation::current::require_in_txn(
            txn,
            &self.f,
            offer.owner_term.licence,
            offer.owner_term.generation,
            None,
        )?;
        let now = world.world_ic_now();
        if now.clock != offer.offered_at_ic.clock
            || now.at < offer.offered_at_ic.at
            || world.now() < offer.offered_at_utc
        {
            return Err(PublicationError::Mismatch);
        }
        let e = world
            .module_state::<mush_economy::EconomyStore>()
            .ok_or(PublicationError::Unavailable)?;
        // This is the actual bounded native book (maximum 65536), not an
        // unavailable-as-empty scan. A seller consent is accepted only once,
        // including after refund; another attempt needs a new signed offer.
        if e.pending_charter_histories()
            .map_err(bad)?
            .any(|h| h.consent().terms.offer == offer.operation)
        {
            return Err(PublicationError::Mismatch);
        }
        let previous = txn
            .get(&self.f.charter_pending_heads, &offer.charter.key())
            .map_err(bad)?
            .cloned();
        let (sequence, previous_ref) = match previous.as_ref() {
            None => (1, None),
            Some(head) => {
                let row = txn
                    .get(&self.f.charter_pending, &head.current.operation.to_hex())
                    .map_err(bad)?
                    .cloned()
                    .ok_or(PublicationError::Missing)?;
                head.matches(&row)?;
                if matches!(row.state, PendingState::Funded {})
                    || e.pending_charter_history(row.operation).map_err(bad)? != &row.economic
                {
                    return Err(PublicationError::Mismatch);
                }
                (
                    row.sequence
                        .checked_add(1)
                        .ok_or(PublicationError::Capacity)?,
                    Some(row.reference()?),
                )
            }
        };
        if sequence > MAX_PENDING_HISTORY {
            return Err(PublicationError::Capacity);
        }
        let claims = world
            .module_state::<crate::RollStore>()
            .ok_or(PublicationError::Unavailable)?
            .charter_pending_claims(offer.charter)
            .map_err(bad)?;
        let admission = self.admission(
            ctx,
            offer.charter,
            offer.seller,
            offer.buyer,
            offer.seller_account,
            offer.expires_at_utc,
            Some(claims.clone()),
        )?;
        if admission.title.revision != offer.title_revision
            || admission.payout.instrument != offer.instrument
        {
            return Err(PublicationError::Mismatch);
        }
        let fence = self.fence(inv, ctx, Mode::Admission(Box::new(admission)))?;
        let native = ctx
            .prepare_owner(
                self.economy.as_ref().ok_or(PublicationError::Unavailable)?,
                EconomyRequest::CharterPendingFunding(CharterPendingFundingRequest {
                    terms: offer.native()?,
                    payer,
                }),
            )
            .map_err(bad)?
            .charter_pending()
            .map_err(bad)?;
        let economic = native.history().clone();
        let c = economic.consent();
        if c.operation != ctx.commit_token()
            || c.effect != self.action
            || c.terms != offer.native()?
            || c.payer != payer
            || economic.terminal().is_some()
        {
            return Err(PublicationError::Mismatch);
        }
        let row = PendingRecord {
            format: 1,
            operation: ctx.commit_token(),
            action: self.action.clone(),
            sequence,
            previous: previous_ref,
            offer: offer.operation,
            offer_digest: offer.digest()?,
            charter: offer.charter,
            ownership: offer.ownership.clone(),
            owner_term: offer.owner_term.clone(),
            accepted_at_ic: c.accepted_at_ic.clone(),
            deadline: c.deadline.clone(),
            claim_observation: claims,
            economic,
            state: PendingState::Funded {},
        };
        row.matches_offer(&offer)?;
        let head = PendingHead::of(&row)?;
        self.roots(
            ctx,
            vec![history::offer_root(&offer)?],
            vec![history::funding_root(&row, &offer)?],
            fence,
        )?;
        txn.insert(&self.f.charter_pending, &row.key(), row.clone())
            .map_err(bad)?;
        if previous.is_some() {
            txn.replace(&self.f.charter_pending_heads, &head.key(), head.clone())
                .map_err(bad)?;
        } else {
            txn.insert(&self.f.charter_pending_heads, &head.key(), head.clone())
                .map_err(bad)?;
        }
        if txn.get(&self.f.charter_pending, &row.key()).map_err(bad)? != Some(&row)
            || txn
                .get(&self.f.charter_pending_heads, &head.key())
                .map_err(bad)?
                != Some(&head)
        {
            return Err(PublicationError::Mismatch);
        }
        ctx.reserve_charter_pending_completion(txn).map_err(bad)?;
        Ok(offer)
    }
    fn terminal(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<PendingOffer, PublicationError> {
        let intent = self.runtime.intent(inv, &self.action)?;
        intent.fresh(ctx.world().map_err(bad)?)?;
        let before = txn
            .get(&self.f.charter_pending, &intent.funding.to_hex())
            .map_err(bad)?
            .cloned()
            .ok_or(PublicationError::Missing)?;
        let head = txn
            .get(&self.f.charter_pending_heads, &before.charter.key())
            .map_err(bad)?
            .cloned()
            .ok_or(PublicationError::Missing)?;
        head.matches(&before)?;
        if !matches!(before.state, PendingState::Funded {})
            || before.reference()? != intent.pending
            || intent.reason.settles() != (self.phase == 2)
        {
            return Err(PublicationError::Mismatch);
        }
        let offer = txn
            .get(&self.f.charter_pending_offers, &before.offer.to_hex())
            .map_err(bad)?
            .cloned()
            .ok_or(PublicationError::Missing)?;
        before.matches_offer(&offer)?;
        let world = ctx.world().map_err(bad)?;
        let e = world
            .module_state::<mush_economy::EconomyStore>()
            .ok_or(PublicationError::Unavailable)?;
        if e.pending_charter_history(before.operation).map_err(bad)? != &before.economic {
            return Err(PublicationError::Mismatch);
        }
        let birth = txn
            .get(&self.f.operations, &offer.birth.to_hex())
            .map_err(bad)?
            .cloned()
            .ok_or(PublicationError::Missing)?;
        offer.matches_birth(&birth)?;
        // A retained terminal requires current physical title and immutable
        // ownership, but does not borrow current source/term/issuer eligibility.
        let ownership = crate::commerce::charter_transfer::history::current_in_txn(
            txn,
            world,
            &self.f,
            &birth,
            offer.seller,
        )?;
        if ownership != before.ownership {
            return Err(PublicationError::Mismatch);
        }
        let next_key = CharterTransfer::key_for(
            before.charter,
            ownership
                .generation
                .checked_add(1)
                .ok_or(PublicationError::Capacity)?,
        );
        if txn
            .get(&self.f.charter_transfers, &next_key)
            .map_err(bad)?
            .is_some()
        {
            return Err(PublicationError::Mismatch);
        }
        let mut old = vec![
            history::offer_root(&offer)?,
            history::funding_root(&before, &offer)?,
        ];
        if let Reason::RegistryRevoked {
            sanction,
            licence,
            generation,
            term_digest,
        } = &intent.reason
        {
            use crate::commerce::registry_revocation::records::{AcceptedTerm, RevokedTerm};
            let status = txn
                .get(
                    &self.f.revoked_terms,
                    &RevokedTerm::key_for(before.owner_term.licence, before.owner_term.generation),
                )
                .map_err(bad)?
                .cloned()
                .ok_or(PublicationError::Missing)?;
            let op = txn
                .get(&self.f.registry_revocations, &sanction.to_hex())
                .map_err(bad)?
                .cloned()
                .ok_or(PublicationError::Missing)?;
            status.matches(&op)?;
            let AcceptedTerm::Owner { term } = &op.term else {
                return Err(PublicationError::Mismatch);
            };
            before.owner_term.matches(term)?;
            if op.charter != before.charter
                || *licence != term.licence.get()
                || *generation != term.generation.get()
                || *term_digest != before.owner_term.digest
            {
                return Err(PublicationError::Mismatch);
            }
            old.push(op.root()?);
        } else {
            // A concurrently committed sanction wins over ordinary settlement;
            // the exact typed absence is retained in this transaction's CAS.
            crate::commerce::registry_revocation::current::require_in_txn(
                txn,
                &self.f,
                before.owner_term.licence,
                before.owner_term.generation,
                None,
            )?;
        }
        let decision = CharterPendingDecisionV1 {
            format: 1,
            operation: ctx.commit_token(),
            action: self.action.clone(),
            actor: inv.actor(),
            at_ic: world.world_ic_now(),
            posted_at_utc: world.now(),
            reason: intent.reason.clone(),
        };
        let fence = self.fence(
            inv,
            ctx,
            Mode::Service {
                runtime: Box::new(self.runtime.clone()),
                intent: Box::new(intent),
            },
        )?;
        let native = ctx
            .prepare_owner(
                self.economy.as_ref().ok_or(PublicationError::Unavailable)?,
                EconomyRequest::CharterPendingTerminal(CharterPendingTerminalRequest {
                    funding: before.operation,
                    decision: decision.clone(),
                }),
            )
            .map_err(bad)?
            .charter_pending()
            .map_err(bad)?;
        let economic = native.history().clone();
        if economic.consent() != before.economic.consent()
            || economic.funding() != before.economic.funding()
            || economic
                .terminal()
                .is_none_or(|t| t.decision() != &decision)
        {
            return Err(PublicationError::Mismatch);
        }
        let transfer = if decision.reason.settles() {
            Some(CharterTransfer {
                format: 2,
                operation: ctx.commit_token(),
                action: self.action.clone(),
                charter: before.charter,
                generation: ownership
                    .generation
                    .checked_add(1)
                    .ok_or(PublicationError::Capacity)?,
                birth: offer.birth,
                predecessor: ownership.clone(),
                offer: offer.operation,
                offer_digest: offer.digest()?,
                accepted_at_ic: before.accepted_at_ic.clone(),
                economic: TransferEconomic::Pending(Box::new(PendingEconomic {
                    pending: economic.clone(),
                })),
            })
        } else {
            None
        };
        let mut row = before.clone();
        row.economic = economic;
        row.state = match &transfer {
            Some(t) => PendingState::Settled {
                terminal: ctx.commit_token(),
                transfer: t.ownership()?,
            },
            None => PendingState::Refunded {
                terminal: ctx.commit_token(),
                reason: decision.reason.clone(),
            },
        };
        row.matches_offer(&offer)?;
        let head = PendingHead::of(&row)?;
        let mut added = vec![history::terminal_root(&row, &offer)?];
        if let Some(t) = &transfer {
            added.push(history::transfer_root(t, &offer, &row)?);
        }
        self.roots(ctx, old, added, fence)?;
        txn.replace(&self.f.charter_pending, &row.key(), row.clone())
            .map_err(bad)?;
        txn.replace(&self.f.charter_pending_heads, &head.key(), head.clone())
            .map_err(bad)?;
        if let Some(t) = transfer {
            let ownership_head = CharterOwnershipHead {
                format: 1,
                charter: t.charter,
                current: t.ownership()?,
            };
            txn.insert(&self.f.charter_transfers, &t.key(), t.clone())
                .map_err(bad)?;
            if ownership.generation == 1 {
                txn.insert(
                    &self.f.charter_ownership,
                    &ownership_head.key(),
                    ownership_head.clone(),
                )
                .map_err(bad)?;
            } else {
                txn.replace(
                    &self.f.charter_ownership,
                    &ownership_head.key(),
                    ownership_head.clone(),
                )
                .map_err(bad)?;
            }
            if txn.get(&self.f.charter_transfers, &t.key()).map_err(bad)? != Some(&t)
                || txn
                    .get(&self.f.charter_ownership, &ownership_head.key())
                    .map_err(bad)?
                    != Some(&ownership_head)
            {
                return Err(PublicationError::Mismatch);
            }
        }
        if txn.get(&self.f.charter_pending, &row.key()).map_err(bad)? != Some(&row)
            || txn
                .get(&self.f.charter_pending_heads, &head.key())
                .map_err(bad)?
                != Some(&head)
        {
            return Err(PublicationError::Mismatch);
        }
        ctx.close_charter_pending_completion(txn, before.operation)
            .map_err(bad)?;
        Ok(offer)
    }
    fn prepare(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, PublicationError> {
        self.parse(inv)?;
        self.ready.check()?;
        let offer = match self.phase {
            0 => self.offer(inv, txn, ctx)?,
            1 => self.fund(inv, txn, ctx)?,
            2 | 3 => self.terminal(inv, txn, ctx)?,
            _ => return Err(PublicationError::Invalid),
        };
        Ok(AuthoritativeActionOutput::new(RulesValue::new(self.outcome.clone(),json!({"operation":ctx.commit_token(),"offer":offer.operation,"charter":offer.charter})).map_err(bad)?)
            .with_definitions(vec![self.action.clone(),self.outcome.clone(),self.retention.clone()])
            .with_eligible_witnesses(vec![inv.principal().clone()]))
    }
}
impl ActionProvider for Provider {
    fn validate_invocation(&self, inv: &ActionInvocation) -> Result<(), ActionError> {
        self.parse(inv).map_err(|_| self.refusal.clone())
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
            || self.ready.check().is_err()
            || !service::live(&e.world, q.actor())
            || q.projected_subject() != &PerceivedRef::SelfActor
            || q.truth_subject() != &(mush_core::SubjectKey::Object { id: q.actor() })
            || e.world
                .module_state::<crate::RollStore>()
                .is_none_or(|r| r.commerce_ready().is_err())
        {
            return Ok(None);
        }
        let fail = || self.refusal.clone();
        let defs = e.registry.definitions().ok_or_else(fail)?;
        let mut guards = e
            .registry
            .server_guard_set_builder(mush_core::RevisionSet::default())
            .map_err(|_| fail())?;
        for d in [&self.action, &self.retention] {
            guards.add_definition(d.clone(), defs).map_err(|_| fail())?;
        }
        let (fixed, fields) = if self.phase >= 2 {
            let Some(intent) = self.runtime.projected() else {
                return Ok(None);
            };
            if intent.actor != q.actor()
                || intent.action != self.action
                || intent.fresh(&e.world).is_err()
            {
                return Ok(None);
            }
            guards
                .add(intent.guard().map_err(|_| fail())?)
                .map_err(|_| fail())?;
            (intent.parameters(), Vec::new())
        } else if self.phase == 1 {
            (json!({}), vec![("offer", 32), ("payer", 20)])
        } else {
            (
                json!({}),
                vec![
                    ("charter", 10),
                    ("buyer", 10),
                    ("seller_account", 20),
                    ("price_minor", 20),
                    ("valid_for_seconds", 5),
                ],
            )
        };
        let parameters = fields
            .into_iter()
            .map(|(id, maximum_bytes)| {
                Ok(mush_core::ProjectedParameter {
                    id: mush_core::mechanics::CanonicalId::new(id).map_err(|_| fail())?,
                    required: true,
                    kind: mush_core::ProjectedParameterKind::Text { maximum_bytes },
                })
            })
            .collect::<Result<Vec<_>, ActionError>>()?;
        let (key, label) = match self.phase {
            0 => ("roll-offer-pending-charter", "Offer a charter transfer"),
            1 => ("roll-fund-pending-charter", "Fund a charter transfer"),
            2 => ("roll-settle-pending-charter", "Settle a charter transfer"),
            _ => ("roll-refund-pending-charter", "Refund a charter transfer"),
        };
        Ok(Some(mush_core::ProviderOfferDraft {fixed_parameters:fixed,
            parameters:mush_core::ProjectedParameterSchema::new(parameters).map_err(|_|fail())?,targets:mush_core::ProjectedTargetSchema::None,
            required_channels:Vec::new(),expires:None,presentation:mush_core::ActionPresentation::new(key,label,
                "A named buyer may fund the published title offer; retained claims keep their captured warrants.").map_err(|_|fail())?,guards:guards.finish().map_err(|_|fail())?}))
    }
}
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    f: CommerceFamilies,
    definitions: [DefinitionRef; 8],
    config: PendingCharterTransferConfig,
) -> Result<Readiness, ModError> {
    if reg.module() != crate::NS || definitions[..4] != config.actions()? {
        return Err(ModError::msg("pending charter registration differs"));
    }
    let ready = Readiness::default();
    let runtime = service::Runtime::default();
    for phase in 0..4 {
        let (Some(action), Some(outcome)) = (definitions.get(phase), definitions.get(phase + 4))
        else {
            return Err(ModError::msg("pending charter registration differs"));
        };
        let action = action.clone();
        let roots = reg.request_owner_participant::<RootParticipant>(action.clone())?;
        let economy = if phase == 0 {
            None
        } else {
            Some(reg.request_owner_participant::<InitialFilingParticipant>(action.clone())?)
        };
        reg.action_provider(
            action.clone(),
            Provider {
                phase,
                action,
                outcome: outcome.clone(),
                retention: crate::commerce::authored_retirement::retention_reference()?,
                f: f.clone(),
                roots,
                economy,
                ready: ready.clone(),
                runtime: runtime.clone(),
                config: config.clone(),
                refusal: ActionError::new(
                    ActionErrorCode::ProviderRejected,
                    false,
                    "The charter transfer is unavailable or stale.",
                )
                .map_err(|e| ModError::msg(e.to_string()))?,
            },
        )?;
    }
    service::register(reg, runtime, ready.clone(), f, config)?;
    Ok(ready)
}

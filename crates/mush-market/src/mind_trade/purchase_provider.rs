//! The closed purchase consumes an acknowledged quote and derives both owners'
//! images from actual E4 outputs. No request contains accounts, prices, terms,
//! participants, licence IDs, source identities, or terminal targets.

use std::collections::BTreeMap;
use std::sync::Arc;

use mush_core::action::{
    ActionError, ActionErrorCode, ActionInvocation, ActionProvider, AuthoritativeActionOutput,
    PerceivedRef, PrincipalId,
};
use mush_core::mechanics::{DefinitionDescriptor, DefinitionRegistryBuilder, OwnerId};
use mush_core::participant::ParticipantHandle;
use mush_core::read_service::OwnerReadHandle;
use mush_core::{
    ActionTxn, CommitToken, DefinitionRef, ModError, PrepareCtx, Registrar, RulesValue,
    StateFamilyHandle,
};
use mush_economy::managed_purchase::{OriginalPurchase, PurchasePayout, PurchaseTerminal};
use mush_economy::registry_filing::{EconomyRequest, InitialFilingParticipant};
use mush_roll::commerce::purchase::{PurchaseParticipants, PurchasePhase};
use mush_roll::commerce::purchase_writer::{
    OriginalPurchaseTerms, OriginalPurchaseWriter, PurchaseQuoteIdentity, RegisteredPurchaseWriter,
};
use mush_roll::commerce::sale_reader::{
    OriginalSaleFacts, OriginalSaleReader, OriginalSaleRequest,
};
use serde::Deserialize;
use serde_json::json;

use super::purchase_guard::PurchaseGuard;
use super::purchase_records::{self, FUND, GRANT, Purchase, PurchaseReceipt, REFUND};
use super::records::{MindQuote, OriginalSubject, Row};
use super::{Families, OriginalQuoteConfig};

const OUTCOME: &str = "original_mind_purchase_recorded";

/// One composition supplies this same writer and reference set to Roll, Market
/// and Economy. Actual registrations validate every grant and private handle.
#[derive(Clone, Debug)]
pub struct OriginalPurchaseConfig {
    pub(crate) quotes: OriginalQuoteConfig,
    pub(super) participants: PurchaseParticipants,
    pub(super) writer: OriginalPurchaseWriter,
}
impl OriginalPurchaseConfig {
    pub fn new(
        quotes: OriginalQuoteConfig,
        participants: PurchaseParticipants,
        writer: OriginalPurchaseWriter,
    ) -> Result<Self, ModError> {
        // Reconstruct all closed declarations now, so malformed configuration
        // fails before any provider exists or funds can be held.
        let value = Self {
            quotes,
            participants,
            writer,
        };
        value.actions()?;
        Ok(value)
    }
    /// Exact host composition references; never a participant handle or grant.
    pub fn participants(&self) -> &PurchaseParticipants {
        &self.participants
    }
    pub fn actions(&self) -> Result<[DefinitionRef; 3], ModError> {
        [FUND, GRANT, REFUND]
            .map(|id| {
                DefinitionRegistryBuilder::new()
                    .register(
                        OwnerId::new(crate::NS).map_err(super::error)?,
                        purchase_records::descriptor_for(
                            id,
                            &self.quotes.manifest,
                            &self.participants,
                            self.quotes.protocol(),
                        )
                        .map_err(super::error)?,
                    )
                    .map_err(super::error)
            })
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?
            .try_into()
            .map_err(|_| ModError::msg("purchase action set differs"))
    }
    pub(crate) fn declarations(&self) -> Result<Vec<DefinitionDescriptor>, ModError> {
        let mut out = [FUND, GRANT, REFUND]
            .into_iter()
            .map(|id| {
                purchase_records::descriptor_for(
                    id,
                    &self.quotes.manifest,
                    &self.participants,
                    self.quotes.protocol(),
                )
                .map_err(super::error)
            })
            .collect::<Result<Vec<_>, _>>()?;
        out.push(super::descriptor(
            "outcome",
            OUTCOME,
            json!({"format":1,"fields":["kind","operation","commit"]}),
        )?);
        Ok(out)
    }
}

#[derive(Clone)]
pub(crate) struct PurchaseFamilies {
    pub(super) operations: StateFamilyHandle<CommitToken, Purchase>,
    pub(super) receipts: StateFamilyHandle<CommitToken, PurchaseReceipt>,
}

pub(crate) fn register(
    reg: &mut Registrar<'_>,
    config: &OriginalPurchaseConfig,
    offers: Families,
    gate: Arc<dyn crate::VenueGate>,
) -> Result<PurchaseFamilies, ModError> {
    if reg.module() != crate::NS {
        return Err(ModError::msg("purchase providers are Market owned"));
    }
    register_writer_binding(reg, config)?;
    let families = PurchaseFamilies {
        operations: super::family(reg)?,
        receipts: super::family(reg)?,
    };
    let guard = PurchaseGuard::new(config.quotes.clone(), gate)?;
    let outcome = reg.definition(super::descriptor(
        "outcome",
        OUTCOME,
        json!({"format":1,"fields":["kind","operation","commit"]}),
    )?)?;
    let actions = config.actions()?;
    for action in &actions {
        let actual = reg.definition(
            purchase_records::descriptor_for(
                action.key().id.as_str(),
                &config.quotes.manifest,
                &config.participants,
                config.quotes.protocol(),
            )
            .map_err(super::error)?,
        )?;
        if &actual != action {
            return Err(ModError::msg("purchase action definition differs"));
        }
        let economy = reg.request_owner_participant::<InitialFilingParticipant>(action.clone())?;
        let writer = config.writer.for_action(
            reg,
            action,
            &config.quotes.manifest,
            &config.participants,
            offers.reader.clone(),
        )?;
        let reader = offers.reader.clone();
        let refusal = ActionError::new(
            ActionErrorCode::ProviderRejected,
            false,
            "The original personal purchase is unavailable or its terms have changed.",
        )
        .map_err(super::error)?;
        reg.action_provider(
            action.clone(),
            Provider {
                action: action.clone(),
                outcome: outcome.clone(),
                config: config.clone(),
                offers: offers.clone(),
                families: families.clone(),
                economy,
                writer,
                reader,
                guard: guard.clone(),
                refusal,
            },
        )?;
    }
    reg.original_mind_purchase_capacity(
        config.quotes.manifest.clone(),
        config.participants.ordered(),
        actions,
    )?;
    super::purchase_service::register(reg, config, offers, families.clone(), guard)?;
    Ok(families)
}

/// Definition freeze runs after every module's registration and before world
/// hydration. Validate the actual Roll-installed cell there, independently of
/// owner roster order, without accepting a serialized handle or boot fallback.
fn register_writer_binding(
    reg: &mut Registrar<'_>,
    config: &OriginalPurchaseConfig,
) -> Result<(), ModError> {
    let manifest = config.quotes.manifest.clone();
    let participants = config.participants.clone();
    let actions = config.actions()?;
    let writer = config.writer.clone();
    let mut references = vec![
        manifest.clone(),
        participants.economy().clone(),
        participants.retention().clone(),
    ];
    references.extend(actions.iter().cloned());
    references.sort();
    references.dedup();
    let descriptor = super::versioned_descriptor(
        "original_purchase_binding",
        "roll",
        config.quotes.protocol().version(),
        json!({"format":config.quotes.protocol().version(),"protocol":if config.quotes.source_forms {"original_mind_purchase_v2"} else {"original_mind_purchase_v1"}}),
    )?
    .with_references(references.clone());
    let expected = DefinitionRegistryBuilder::new()
        .register(
            OwnerId::new(crate::NS).map_err(super::error)?,
            descriptor.clone(),
        )
        .map_err(super::error)?;
    let payload = descriptor.payload().clone();
    reg.definition_validator(
        mush_core::mechanics::DefinitionKind::new("original_purchase_binding")
            .map_err(super::error)?,
        move |ctx: &mush_core::mechanics::DefinitionValidationContext<'_>| {
            let definition = ctx.definition();
            if definition.reference() != &expected
                || definition.payload() != &payload
                || definition.references() != references.as_slice()
            {
                return Err("paid writer binding definition differs".into());
            }
            for reference in &references {
                ctx.get_exact(reference).map_err(|e| e.to_string())?;
            }
            writer
                .validate_registration(&manifest, &participants, &actions)
                .map_err(|e| e.to_string())
        },
    )?;
    reg.definition(descriptor)?;
    Ok(())
}

struct Provider {
    action: DefinitionRef,
    outcome: DefinitionRef,
    config: OriginalPurchaseConfig,
    offers: Families,
    families: PurchaseFamilies,
    economy: ParticipantHandle<InitialFilingParticipant>,
    writer: RegisteredPurchaseWriter,
    reader: OwnerReadHandle<OriginalSaleReader>,
    guard: PurchaseGuard,
    refusal: ActionError,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FundRequest {
    quote: CommitToken,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TerminalRequest {
    operation: CommitToken,
}
enum Request {
    Fund(CommitToken),
    Grant(CommitToken),
    Refund(CommitToken),
}
impl Provider {
    fn request(&self, inv: &ActionInvocation) -> Result<Request, String> {
        if inv.principal() != &(PrincipalId::Object { id: inv.actor() })
            || inv.subject() != &PerceivedRef::SelfActor
            || !inv.targets().is_empty()
            || inv.idempotency().scope().owner != self.action.key().owner
        {
            return Err("invalid purchase requester".into());
        }
        match self.action.key().id.as_str() {
            FUND => serde_json::from_value::<FundRequest>(inv.parameters().clone())
                .map(|r| Request::Fund(r.quote)),
            GRANT => serde_json::from_value::<TerminalRequest>(inv.parameters().clone())
                .map(|r| Request::Grant(r.operation)),
            REFUND => serde_json::from_value::<TerminalRequest>(inv.parameters().clone())
                .map(|r| Request::Refund(r.operation)),
            _ => return Err("unknown purchase transition".into()),
        }
        .map_err(|e| e.to_string())
    }
    fn current(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
        q: &MindQuote,
    ) -> Result<OriginalSaleFacts, String> {
        q.validate()?;
        let p = &self.config.quotes.policy;
        if q.buyer != inv.actor()
            || q.policy != self.config.quotes.policy_digest()?
            || q.product.protocol() != self.config.quotes.protocol()
            || q.subject.compatibility != self.config.quotes.manifest.digest()
            || q.price != p.price
            || q.instrument != p.instrument
            || q.venue != p.venue
            || q.registry_recipient != p.registry_recipient
            || q.registry_account != p.registry_account
            || q.registry_permille != p.registry_permille
            || q.product.transferable != p.transferable
            || q.product.term_seconds != p.term_seconds
            || q.product.grace_seconds != p.grace_seconds
        {
            return Err("purchase differs from current configured terms".into());
        }
        let facts = ctx
            .read_owner_preimage(
                txn,
                &self.reader,
                OriginalSaleRequest::listed(q.subject.seller, q.subject.owner_licence),
            )
            .map_err(|e| e.to_string())?;
        if facts.requester() != inv.actor()
            || OriginalSubject::capture(&facts) != q.subject
            || q.product.class != facts.minimum()
        {
            return Err("purchase original owner or certificate changed".into());
        }
        ctx.distinct_personal_accounts(q.subject.seller)
            .map_err(|e| e.to_string())?;
        let world = ctx.world().map_err(|e| e.to_string())?;
        self.guard.current(world, inv.actor())?;
        if world.now() < q.quoted_at_utc || world.now() >= q.expires_at_utc {
            return Err("purchase quote expired".into());
        }
        Ok(facts)
    }
    #[inline(never)]
    fn prepare_funding(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
        quote: CommitToken,
        commit: CommitToken,
    ) -> Result<(CommitToken, &'static str), String> {
        if quote == commit {
            return Err("quote is not acknowledged".into());
        }
        let q = txn
            .get(&self.offers.quotes, &quote)
            .map_err(|e| e.to_string())?
            .cloned()
            .ok_or("quote unavailable")?;
        if q.publication != quote {
            return Err("quote key differs".into());
        }
        let listing = txn
            .get(&self.offers.listings, &q.listing)
            .map_err(|e| e.to_string())?
            .ok_or("listing unavailable")?;
        listing.validate()?;
        if listing.publication != q.listing || listing.digest()? != q.listing_digest {
            return Err("quoted listing changed or was withdrawn".into());
        }
        let facts = self.current(inv, txn, ctx, &q)?;
        let output = ctx
            .prepare_owner(
                &self.economy,
                EconomyRequest::PurchaseFunding(economics(&q)?),
            )
            .and_then(|o| o.funding())
            .map_err(|e| e.to_string())?;
        let allocation = self
            .writer
            .fund(
                ctx,
                txn,
                PurchaseQuoteIdentity {
                    quote,
                    quote_digest: q.digest()?,
                    product_digest: q.product_digest,
                },
                terms(&q),
                &facts,
                &output,
            )
            .map_err(|e| e.to_string())?;
        let row = Purchase::funded(
            q.clone(),
            self.action.clone(),
            self.config.participants.clone(),
            &output,
            &allocation,
        )?;
        txn.insert(&self.families.operations, &commit, row.clone())
            .map_err(|e| e.to_string())?;
        super::index::retire_quotes(
            txn,
            &self.offers,
            &[quote],
            Some(inv.actor()),
            ctx.world().map_err(|e| e.to_string())?.now(),
        )?;
        if txn
            .get(&self.families.operations, &commit)
            .map_err(|e| e.to_string())?
            != Some(&row)
        {
            return Err("funded typed post-image differs".into());
        }
        // The kernel derives all five keys from these actual paired
        // rows and checks the unchanged quote and shared allocator.
        ctx.reserve_original_mind_purchase_completion(txn)
            .map_err(|e| e.to_string())?;
        Ok((commit, "funded"))
    }
    #[inline(never)]
    fn prepare_terminal(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
        request: Request,
    ) -> Result<(CommitToken, &'static str), String> {
        let (operation, grant) = match request {
            Request::Grant(id) => (id, true),
            Request::Refund(id) => (id, false),
            Request::Fund(_) => return Err("unexpected funding transition".into()),
        };
        let before = txn
            .get(&self.families.operations, &operation)
            .map_err(|e| e.to_string())?
            .cloned()
            .ok_or("funded purchase unavailable")?;
        before.validate()?;
        if before.operation != operation
            || before.phase != (PurchasePhase::Funded {})
            || (inv.actor() != before.quote.buyer && (grant || inv.actor() != mush_core::GOD))
        {
            return Err("purchase terminal authority differs".into());
        }
        let allocation = self
            .writer
            .allocation(txn, operation)
            .map_err(|e| e.to_string())?;
        before.matches_allocation(&allocation)?;
        let facts = if grant {
            if before.participants != self.config.participants {
                return Err("purchase participants are historical".into());
            }
            Some(self.current(inv, txn, ctx, &before.quote)?)
        } else {
            None
        };
        let terminal = PurchaseTerminal {
            operation,
            purchase: economics(&before.quote)?,
            funding: before.funding.clone(),
        };
        let request = if grant {
            EconomyRequest::PurchaseGrant(terminal)
        } else {
            EconomyRequest::PurchaseRefund(terminal)
        };
        let output = ctx
            .prepare_owner(&self.economy, request)
            .and_then(|o| o.terminal())
            .map_err(|e| e.to_string())?;
        let after_allocation = self
            .writer
            .terminal(
                ctx,
                txn,
                &allocation,
                terms(&before.quote),
                facts.as_ref(),
                &output,
            )
            .map_err(|e| e.to_string())?;
        let (after, receipt) = before.terminal(&output)?;
        after.matches_allocation(&after_allocation)?;
        txn.replace(&self.families.operations, &operation, after.clone())
            .map_err(|e| e.to_string())?;
        txn.insert(&self.families.receipts, &operation, receipt.clone())
            .map_err(|e| e.to_string())?;
        if txn
            .get(&self.families.operations, &operation)
            .map_err(|e| e.to_string())?
            != Some(&after)
            || txn
                .get(&self.families.receipts, &operation)
                .map_err(|e| e.to_string())?
                != Some(&receipt)
        {
            return Err("terminal typed post-images differ".into());
        }
        ctx.finish_original_mind_purchase_completion(txn, operation)
            .map_err(|e| e.to_string())?;
        Ok((operation, if grant { "granted" } else { "refunded" }))
    }
    fn prepare(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, String> {
        let commit = ctx.commit_token();
        let (operation, kind) = match self.request(inv)? {
            Request::Fund(quote) => self.prepare_funding(inv, txn, ctx, quote, commit)?,
            request => self.prepare_terminal(inv, txn, ctx, request)?,
        };
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({"kind":kind,"operation":operation,"commit":commit}),
        )
        .map_err(|e| e.to_string())?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_definitions(vec![
                self.outcome.clone(),
                self.config.quotes.manifest.clone(),
            ])
            .with_eligible_witnesses(vec![inv.principal().clone()]))
    }
}

pub(super) fn terms(q: &MindQuote) -> OriginalPurchaseTerms {
    OriginalPurchaseTerms {
        class: q.product.class,
        form: q.product.form,
        transferable: q.product.transferable,
        term_seconds: q.product.term_seconds,
        grace_seconds: q.product.grace_seconds,
    }
}
pub(super) fn economics(q: &MindQuote) -> Result<OriginalPurchase, String> {
    q.validate()?;
    let mut owners = BTreeMap::new();
    for (account, owner) in [
        (
            q.seller_account,
            mush_org::PrincipalId::Object(q.subject.seller),
        ),
        (
            q.registry_account,
            mush_org::PrincipalId::System(q.registry_recipient.clone()),
        ),
        (q.venue.account, q.venue.recipient.clone()),
    ] {
        if owners
            .insert(account, owner.clone())
            .is_some_and(|previous| previous != owner)
        {
            return Err("quote credits one account under different owners".into());
        }
    }
    let payouts = q
        .payouts
        .iter()
        .map(|p| {
            Ok(PurchasePayout {
                account: p.account,
                amount: p.amount,
                owner: owners
                    .get(&p.account)
                    .cloned()
                    .ok_or("unknown quote recipient")?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(OriginalPurchase {
        quote: q.publication,
        quote_digest: q.digest()?,
        product_digest: q.product_digest,
        buyer: q.buyer,
        seller: q.subject.seller,
        charter: q.subject.charter.object(),
        title_revision: q.subject.title_revision,
        payer: q.payer,
        instrument: q.instrument.clone(),
        price: q.price,
        payouts,
        quoted_at_utc: q.quoted_at_utc,
        expires_at_utc: q.expires_at_utc,
    })
}

impl ActionProvider for Provider {
    fn validate_invocation(&self, inv: &ActionInvocation) -> Result<(), ActionError> {
        self.request(inv)
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
        query: &mush_core::LegalActionQuery,
        engine: &mush_core::Engine,
    ) -> Result<Option<mush_core::ProviderOfferDraft>, ActionError> {
        let refund = self.action.key().id.as_str() == REFUND;
        if action != &self.action
            || query.principal() != &(PrincipalId::Object { id: query.actor() })
            || query.truth_subject() != &(mush_core::SubjectKey::Object { id: query.actor() })
            || query.projected_subject() != &PerceivedRef::SelfActor
            || engine.world.get(query.actor()).is_none_or(|o| {
                o.kind != mush_core::ObjKind::Player || o.flags.contains(mush_core::Flags::GOING)
            })
            || (!refund && self.guard.current(&engine.world, query.actor()).is_err())
        {
            return Ok(None);
        }
        let bad = || self.refusal.clone();
        let defs = engine.registry.definitions().ok_or_else(bad)?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(mush_core::RevisionSet::default())
            .map_err(|_| bad())?;
        for definition in [
            &self.action,
            &self.config.quotes.manifest,
            self.config.participants.economy(),
            self.config.participants.retention(),
        ] {
            guards
                .add_definition(definition.clone(), defs)
                .map_err(|_| bad())?;
        }
        if !refund {
            guards
                .add(self.guard.witness(query.actor()))
                .map_err(|_| bad())?;
        }
        let id = if self.action.key().id.as_str() == FUND {
            "quote"
        } else {
            "operation"
        };
        Ok(Some(mush_core::ProviderOfferDraft {
            fixed_parameters: json!({}),
            parameters: mush_core::ProjectedParameterSchema::new(vec![
                mush_core::ProjectedParameter {
                    id: mush_core::mechanics::CanonicalId::new(id).map_err(|_| bad())?,
                    required: true,
                    kind: mush_core::ProjectedParameterKind::Text { maximum_bytes: 32 },
                },
            ])
            .map_err(|_| bad())?,
            targets: mush_core::ProjectedTargetSchema::None,
            required_channels: vec![],
            expires: None,
            presentation: mush_core::ActionPresentation::new(
                "market-original-mind-purchase",
                "Complete an original mind purchase",
                "Use the exact quoted terms, or refund this purchase's escrow.",
            )
            .map_err(|_| bad())?,
            guards: guards.finish().map_err(|_| bad())?,
        }))
    }
}

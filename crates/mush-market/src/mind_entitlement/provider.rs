//! Actual two-owner funding and terminal publication. Only opaque E4 outputs
//! can derive Roll's grant; the kernel derives the exact six reserved targets.
use super::codec::Row;
use super::families::Families;
use super::guard::EntitlementGuard;
use super::{EntitlementConfig, EntitlementPurchaseV1, EntitlementQuoteV1};
use super::{actions, offers, publication};
use mush_core::action::{ActionError, ActionInvocation, ActionProvider, AuthoritativeActionOutput};
use mush_core::participant::ParticipantHandle;
use mush_core::{ActionTxn, CommitToken, DefinitionRef, PrepareCtx, RulesValue};
use mush_economy::managed_entitlement::{EntitlementProtocol, EntitlementTerminal};
use mush_economy::registry_filing::{EconomyRequest, InitialFilingParticipant};
use mush_roll::commerce::entitlement::writer::{
    PreparedEntitlementSale, RegisteredEntitlementWriter,
};
use mush_roll::commerce::purchase::PurchasePhase;
use serde::Deserialize;
#[derive(Clone, Copy)]
pub(super) enum Kind {
    Fund,
    Grant,
    Refund,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fund {
    quote: CommitToken,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Terminal {
    operation: CommitToken,
}
enum Request {
    Fund(CommitToken),
    Terminal(CommitToken, bool),
}
pub(super) struct Provider {
    pub action: DefinitionRef,
    pub outcome: DefinitionRef,
    pub protocol: EntitlementProtocol,
    pub kind: Kind,
    pub config: EntitlementConfig,
    pub families: Families,
    pub economy: ParticipantHandle<InitialFilingParticipant>,
    pub writer: RegisteredEntitlementWriter,
    pub guard: EntitlementGuard,
    pub refusal: ActionError,
}
impl Provider {
    fn request(&self, inv: &ActionInvocation) -> Result<Request, String> {
        actions::requester(inv, &self.action)?;
        match self.kind {
            Kind::Fund => serde_json::from_value::<Fund>(inv.parameters().clone())
                .map(|r| Request::Fund(r.quote)),
            Kind::Grant | Kind::Refund => {
                serde_json::from_value::<Terminal>(inv.parameters().clone())
                    .map(|r| Request::Terminal(r.operation, matches!(self.kind, Kind::Grant)))
            }
        }
        .map_err(|e| { let m = e.to_string(); if std::env::var_os("M7_CONDUCT_DIAG").is_some() { eprintln!("M7_CONDUCT_DIAG entitlement provider.rs:{}: {m}", line!()); } m }) /* TEMP M7 diag */
    }
    fn current(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
        q: &EntitlementQuoteV1,
    ) -> Result<PreparedEntitlementSale, String> {
        q.validate()?;
        if q.buyer != inv.actor()
            || q.product.protocol != self.protocol
            || q.subject.compatibility != self.config.manifest.digest()
        {
            return Err("entitlement quote identity is historical".into());
        }
        let listing = txn
            .get(&self.families.listings, &q.listing)
            .map_err(|e| { let m = e.to_string(); if std::env::var_os("M7_CONDUCT_DIAG").is_some() { eprintln!("M7_CONDUCT_DIAG entitlement provider.rs:{}: {m}", line!()); } m }) /* TEMP M7 diag */?
            .cloned()
            .ok_or("entitlement listing missing")?;
        if listing.publication != q.listing
            || listing.digest()? != q.listing_digest
            || listing.subject != q.subject
            || listing.product != q.product
            || listing.royalty != q.royalty
            || listing.price != q.price
            || listing.instrument != q.instrument
            || listing.registry != q.registry
            || listing.venue != q.venue
            || listing.policy != q.policy
        {
            return Err("entitlement consent differs from its listing".into());
        }
        let observation = self
            .writer
            .observe_purchase(ctx, txn, q.subject.seller, q.subject.owner_licence)
            .map_err(|e| { let m = e.to_string(); if std::env::var_os("M7_CONDUCT_DIAG").is_some() { eprintln!("M7_CONDUCT_DIAG entitlement provider.rs:{}: {m}", line!()); } m }) /* TEMP M7 diag */?;
        offers::current_listing(
            &self.config,
            self.protocol,
            &listing,
            observation.facts(),
            inv.actor(),
        )?;
        let world = ctx.world().map_err(|e| { let m = e.to_string(); if std::env::var_os("M7_CONDUCT_DIAG").is_some() { eprintln!("M7_CONDUCT_DIAG entitlement provider.rs:{}: {m}", line!()); } m }) /* TEMP M7 diag */?;
        self.guard.current(world, inv.actor())?;
        if world.now() < q.quoted_at_utc || world.now() >= q.expires_at_utc {
            return Err("entitlement quote expired".into());
        }
        Ok(observation)
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
            .get(&self.families.quotes, &quote)
            .map_err(|e| { let m = e.to_string(); if std::env::var_os("M7_CONDUCT_DIAG").is_some() { eprintln!("M7_CONDUCT_DIAG entitlement provider.rs:{}: {m}", line!()); } m }) /* TEMP M7 diag */?
            .cloned()
            .ok_or("quote unavailable")?;
        if q.publication != quote {
            return Err("quote key differs".into());
        }
        let observed = self.current(inv, txn, ctx, &q)?;
        let economics = q.economic_subject()?;
        self.writer
            .preflight_funding(ctx, txn, &economics, &publication::terms(&q), &observed)
            .map_err(|e| { let m = e.to_string(); if std::env::var_os("M7_CONDUCT_DIAG").is_some() { eprintln!("M7_CONDUCT_DIAG entitlement provider.rs:{}: {m}", line!()); } m }) /* TEMP M7 diag */?;
        let output = ctx
            .prepare_owner(
                &self.economy,
                EconomyRequest::EntitlementFunding(economics.clone()),
            )
            .and_then(|o| o.entitlement_funding())
            .map_err(|e| { let m = e.to_string(); if std::env::var_os("M7_CONDUCT_DIAG").is_some() { eprintln!("M7_CONDUCT_DIAG entitlement provider.rs:{}: {m}", line!()); } m }) /* TEMP M7 diag */?;
        let allocation = self
            .writer
            .fund(
                ctx,
                txn,
                &economics,
                &publication::terms(&q),
                &observed,
                &output,
            )
            .map_err(|e| { let m = e.to_string(); if std::env::var_os("M7_CONDUCT_DIAG").is_some() { eprintln!("M7_CONDUCT_DIAG entitlement provider.rs:{}: {m}", line!()); } m }) /* TEMP M7 diag */?;
        let row = EntitlementPurchaseV1::funded(
            q,
            self.action.clone(),
            self.config.participants.clone(),
            &output,
            &allocation,
        )?;
        txn.insert(&self.families.operations, &commit, row.clone())
            .map_err(|e| { let m = e.to_string(); if std::env::var_os("M7_CONDUCT_DIAG").is_some() { eprintln!("M7_CONDUCT_DIAG entitlement provider.rs:{}: {m}", line!()); } m }) /* TEMP M7 diag */?;
        self.families.consume_quote(txn, quote)?;
        if txn
            .get(&self.families.operations, &commit)
            .map_err(|e| { let m = e.to_string(); if std::env::var_os("M7_CONDUCT_DIAG").is_some() { eprintln!("M7_CONDUCT_DIAG entitlement provider.rs:{}: {m}", line!()); } m }) /* TEMP M7 diag */?
            != Some(&row)
        {
            return Err("funded entitlement post-image differs".into());
        }
        ctx.reserve_mind_entitlement_purchase_completion(txn)
            .map_err(|e| { let m = e.to_string(); if std::env::var_os("M7_CONDUCT_DIAG").is_some() { eprintln!("M7_CONDUCT_DIAG entitlement provider.rs:{}: {m}", line!()); } m }) /* TEMP M7 diag */?;
        Ok((commit, "funded"))
    }
    #[inline(never)]
    fn prepare_terminal(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
        operation: CommitToken,
        grant: bool,
    ) -> Result<(CommitToken, &'static str), String> {
        let before = txn
            .get(&self.families.operations, &operation)
            .map_err(|e| { let m = e.to_string(); if std::env::var_os("M7_CONDUCT_DIAG").is_some() { eprintln!("M7_CONDUCT_DIAG entitlement provider.rs:{}: {m}", line!()); } m }) /* TEMP M7 diag */?
            .cloned()
            .ok_or("funded entitlement unavailable")?;
        before.validate()?;
        if before.operation != operation
            || before.phase != (PurchasePhase::Funded {})
            || before.quote.product.protocol != self.protocol
            || (inv.actor() != before.quote.buyer && (grant || inv.actor() != mush_core::GOD))
        {
            return Err("entitlement terminal authority differs".into());
        }
        let allocation = self
            .writer
            .allocation(txn, operation)
            .map_err(|e| { let m = e.to_string(); if std::env::var_os("M7_CONDUCT_DIAG").is_some() { eprintln!("M7_CONDUCT_DIAG entitlement provider.rs:{}: {m}", line!()); } m }) /* TEMP M7 diag */?;
        before.matches_allocation(&allocation)?;
        let observed = if grant {
            if before.participants != self.config.participants {
                return Err("entitlement participants are historical".into());
            }
            Some(self.current(inv, txn, ctx, &before.quote)?)
        } else {
            None
        };
        let terminal = EntitlementTerminal {
            operation,
            purchase: before.quote.economic_subject()?,
            funding: before.funding.clone(),
        };
        let request = if grant {
            EconomyRequest::EntitlementGrant(terminal)
        } else {
            EconomyRequest::EntitlementRefund(terminal)
        };
        let output = ctx
            .prepare_owner(&self.economy, request)
            .and_then(|o| o.entitlement_terminal())
            .map_err(|e| { let m = e.to_string(); if std::env::var_os("M7_CONDUCT_DIAG").is_some() { eprintln!("M7_CONDUCT_DIAG entitlement provider.rs:{}: {m}", line!()); } m }) /* TEMP M7 diag */?;
        let after_allocation = self
            .writer
            .terminal(
                ctx,
                txn,
                inv.actor(),
                &allocation,
                &publication::terms(&before.quote),
                observed.as_ref(),
                &output,
            )
            .map_err(|e| { let m = e.to_string(); if std::env::var_os("M7_CONDUCT_DIAG").is_some() { eprintln!("M7_CONDUCT_DIAG entitlement provider.rs:{}: {m}", line!()); } m }) /* TEMP M7 diag */?;
        let (after, receipt) = before.terminal(&output)?;
        after.matches_allocation(&after_allocation)?;
        txn.replace(&self.families.operations, &operation, after.clone())
            .map_err(|e| { let m = e.to_string(); if std::env::var_os("M7_CONDUCT_DIAG").is_some() { eprintln!("M7_CONDUCT_DIAG entitlement provider.rs:{}: {m}", line!()); } m }) /* TEMP M7 diag */?;
        txn.insert(&self.families.receipts, &operation, receipt.clone())
            .map_err(|e| { let m = e.to_string(); if std::env::var_os("M7_CONDUCT_DIAG").is_some() { eprintln!("M7_CONDUCT_DIAG entitlement provider.rs:{}: {m}", line!()); } m }) /* TEMP M7 diag */?;
        if txn
            .get(&self.families.operations, &operation)
            .map_err(|e| { let m = e.to_string(); if std::env::var_os("M7_CONDUCT_DIAG").is_some() { eprintln!("M7_CONDUCT_DIAG entitlement provider.rs:{}: {m}", line!()); } m }) /* TEMP M7 diag */?
            != Some(&after)
            || txn
                .get(&self.families.receipts, &operation)
                .map_err(|e| { let m = e.to_string(); if std::env::var_os("M7_CONDUCT_DIAG").is_some() { eprintln!("M7_CONDUCT_DIAG entitlement provider.rs:{}: {m}", line!()); } m }) /* TEMP M7 diag */?
                != Some(&receipt)
        {
            return Err("terminal entitlement post-image differs".into());
        }
        ctx.finish_mind_entitlement_purchase_completion(txn, operation)
            .map_err(|e| { let m = e.to_string(); if std::env::var_os("M7_CONDUCT_DIAG").is_some() { eprintln!("M7_CONDUCT_DIAG entitlement provider.rs:{}: {m}", line!()); } m }) /* TEMP M7 diag */?;
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
            Request::Terminal(operation, grant) => {
                self.prepare_terminal(inv, txn, ctx, operation, grant)?
            }
        };
        let outcome = RulesValue::new(
            self.outcome.clone(),
            serde_json::json!({"kind":kind,"operation":operation,"commit":commit}),
        )
        .map_err(|e| { let m = e.to_string(); if std::env::var_os("M7_CONDUCT_DIAG").is_some() { eprintln!("M7_CONDUCT_DIAG entitlement provider.rs:{}: {m}", line!()); } m }) /* TEMP M7 diag */?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_definitions(vec![self.outcome.clone(), self.config.manifest.clone()])
            .with_eligible_witnesses(vec![inv.principal().clone()]))
    }
}
impl ActionProvider for Provider {
    fn validate_invocation(&self, inv: &ActionInvocation) -> Result<(), ActionError> {
        self.request(inv)
            .map(|_| ())
            .map_err(|e| { if std::env::var_os("M7_CONDUCT_DIAG").is_some() { eprintln!("M7_CONDUCT_DIAG entitlement provider.rs validate: {e}"); } self.refusal.clone() }) // TEMP M7 diag
    }
    fn prepare_authoritative_with_context(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        self.prepare(inv, txn, ctx)
            .map_err(|e| { if std::env::var_os("M7_CONDUCT_DIAG").is_some() { eprintln!("M7_CONDUCT_DIAG entitlement provider.rs prepare: {e}"); } self.refusal.clone() }) // TEMP M7 diag
    }
    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &mush_core::LegalActionQuery,
        engine: &mush_core::Engine,
    ) -> Result<Option<mush_core::ProviderOfferDraft>, ActionError> {
        let id = if matches!(self.kind, Kind::Fund) {
            "quote"
        } else {
            "operation"
        };
        let parameter = actions::parameter(
            id,
            mush_core::ProjectedParameterKind::Text { maximum_bytes: 32 },
        )
        .map_err(|_| self.refusal.clone())?;
        actions::offer(
            action,
            &self.action,
            query,
            engine,
            &self.config,
            &self.guard,
            matches!(self.kind, Kind::Refund),
            vec![parameter],
            &self.refusal,
        )
    }
}

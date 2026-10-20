//! Actual listing and quote producers. A listing materializes immutable
//! royalty evidence through Roll; a quote remains unfunded consent data.
use super::codec::{Bounded, Row};
use super::families::Families;
use super::guard::HouseGuard;
use super::records::Cut;
use super::{HouseConfig, HouseListingV1, HouseQuoteV1, HouseSubjectV1};
use super::{accounts, actions};
use mush_core::action::{
    ActionError, ActionErrorCode, ActionInvocation, ActionProvider, AuthoritativeActionOutput,
};
use mush_core::{ActionTxn, CommitToken, DefinitionRef, PrepareCtx, RulesValue};
use mush_economy::managed_entitlement::Recipient;
use mush_roll::commerce::LicenceId;
use mush_roll::commerce::house_catalogue::{HouseSaleFacts, HouseSaleRequest};
use mush_roll::commerce::house_purchase::HouseProtocol;
use mush_roll::commerce::house_purchase::writer::RegisteredHouseWriter;
use serde::Deserialize;
#[derive(Clone, Copy)]
pub(super) enum Kind {
    List,
    Quote,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct List {
    owner_licence: LicenceId,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Quote {
    listing: CommitToken,
}
enum Request {
    List(List),
    Quote(Quote),
}
pub(super) struct Provider {
    pub action: DefinitionRef,
    pub outcome: DefinitionRef,
    pub protocol: HouseProtocol,
    pub kind: Kind,
    pub config: HouseConfig,
    pub families: Families,
    pub writer: Option<RegisteredHouseWriter>,
    pub guard: HouseGuard,
    pub refusal: ActionError,
}
pub(super) fn refusal() -> Result<ActionError, mush_core::ModError> {
    ActionError::new(
        ActionErrorCode::ProviderRejected,
        false,
        "No house mind licence is available under these terms.",
    )
    .map_err(super::error)
}
impl Provider {
    fn request(&self, inv: &ActionInvocation) -> Result<Request, String> {
        actions::requester(inv, &self.action)?;
        match self.kind {
            Kind::List => {
                serde_json::from_value::<List>(inv.parameters().clone()).map(Request::List)
            }
            Kind::Quote => {
                serde_json::from_value::<Quote>(inv.parameters().clone()).map(Request::Quote)
            }
        }
        .map_err(|e| e.to_string())
    }
    fn prepare(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, String> {
        let commit = ctx.commit_token();
        let policy = self
            .config
            .policy_for_protocol(self.protocol)
            .map_err(|e| e.to_string())?;
        let kind = match self.request(inv)? {
            Request::List(request) => {
                let writer = self.writer.as_ref().ok_or("listing writer unavailable")?;
                let observed = writer
                    .observe_listing(ctx, txn, request.owner_licence)
                    .map_err(|e| e.to_string())?;
                let facts = observed.facts();
                if facts.requester() != inv.actor() {
                    return Err("listing subject differs".into());
                }
                let world = ctx.world().map_err(|e| e.to_string())?;
                self.guard.current(world, inv.actor())?;
                let economy = accounts::live(world)?;
                accounts::treasury(economy, &policy.instrument, facts.issuer())?;
                accounts::cut(economy, &policy.instrument, &policy.registry)?;
                accounts::cut(economy, &policy.instrument, &policy.venue)?;
                let row = HouseListingV1 {
                    format: 1,
                    publication: commit,
                    publisher: inv.actor(),
                    subject: HouseSubjectV1::capture(facts),
                    product: self.config.product_for_protocol(self.protocol, facts)?,
                    price: policy.price,
                    instrument: policy.instrument.clone(),
                    venue: policy.venue(),
                    registry: Cut(policy.registry.clone()),
                    policy: self
                        .config
                        .policy_ref_for_protocol(self.protocol)
                        .map_err(|e| e.to_string())?,
                    observed_at_utc: world.now(),
                };
                row.validate()?;
                writer
                    .materialize_listing(ctx, txn, &observed)
                    .map_err(|e| e.to_string())?;
                self.families.add_listing(txn, row)?;
                "listing"
            }
            Request::Quote(request) => {
                if request.listing == commit {
                    return Err("listing is not acknowledged".into());
                }
                let listing = txn
                    .get(&self.families.listings, &request.listing)
                    .map_err(|e| e.to_string())?
                    .cloned()
                    .ok_or("listing unavailable")?;
                if listing.publication != request.listing {
                    return Err("listing key differs".into());
                }
                let facts = ctx
                    .read_owner_preimage(
                        txn,
                        &self.families.reader,
                        HouseSaleRequest::listed(
                            listing.subject.owner_licence,
                            listing.subject.issuer,
                        ),
                    )
                    .map_err(|e| e.to_string())?;
                current_listing(&self.config, self.protocol, &listing, &facts, inv.actor())?;
                let account = ctx.personal_account().map_err(|e| e.to_string())?;
                if account.requester() != inv.actor() {
                    return Err("buyer account binding differs".into());
                }
                let world = ctx.world().map_err(|e| e.to_string())?;
                self.guard.current(world, inv.actor())?;
                let economy = accounts::live(world)?;
                let payer = accounts::personal(economy, &listing.instrument, inv.actor())?;
                if payer.approval.is_some() {
                    return Err("direct personal spending account required".into());
                }
                let treasury =
                    accounts::treasury(economy, &listing.instrument, listing.subject.issuer)?;
                accounts::cut(economy, &listing.instrument, &listing.registry.0)?;
                accounts::cut(economy, &listing.instrument, &listing.venue.cut.0)?;
                let mut q = HouseQuoteV1 {
                    format: 1,
                    publication: commit,
                    listing: request.listing,
                    listing_digest: listing.digest()?,
                    buyer: inv.actor(),
                    subject: listing.subject.clone(),
                    product_digest: listing.product.digest()?,
                    product: listing.product.clone(),
                    price: listing.price,
                    instrument: listing.instrument.clone(),
                    registry: listing.registry.clone(),
                    venue: listing.venue.clone(),
                    remainder: Recipient {
                        account: treasury.id,
                        owner: listing.subject.issuer.principal(),
                    },
                    payer: payer.id,
                    payouts: Bounded(vec![]),
                    policy: listing.policy.clone(),
                    quoted_at_utc: world.now(),
                    expires_at_utc: world
                        .now()
                        .checked_add(policy.quote_seconds)
                        .ok_or("quote expiry overflow")?,
                };
                accounts::fill_payouts(&mut q)?;
                self.families.add_quote(txn, q)?;
                "quote"
            }
        };
        let outcome = RulesValue::new(
            self.outcome.clone(),
            serde_json::json!({"kind":kind,"operation":commit,"commit":commit}),
        )
        .map_err(|e| e.to_string())?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_definitions(vec![
                self.outcome.clone(),
                self.config.manifest.clone(),
                self.config
                    .policy_ref_for_protocol(self.protocol)
                    .map_err(|e| e.to_string())?,
            ])
            .with_eligible_witnesses(vec![inv.principal().clone()]))
    }
}
pub(super) fn current_listing(
    config: &HouseConfig,
    protocol: HouseProtocol,
    l: &HouseListingV1,
    f: &HouseSaleFacts,
    actor: mush_core::ObjId,
) -> Result<(), String> {
    l.validate()?;
    let policy = config
        .policy_for_protocol(protocol)
        .map_err(|e| e.to_string())?;
    let expected = config.product_for_protocol(protocol, f)?;
    if f.requester() != actor
        || l.product.protocol != protocol
        || l.subject != HouseSubjectV1::capture(f)
        || l.product != expected
        || l.price != policy.price
        || l.instrument != policy.instrument
        || l.registry.0 != policy.registry
        || l.venue != policy.venue()
        || l.policy
            != config
                .policy_ref_for_protocol(protocol)
                .map_err(|e| e.to_string())?
    {
        return Err("listing no longer matches actual current house".into());
    }
    Ok(())
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
        use mush_core::ProjectedParameterKind as K;
        let params = match self.kind {
            Kind::List => vec![actions::parameter(
                "owner_licence",
                K::Text { maximum_bytes: 20 },
            )],
            Kind::Quote => vec![actions::parameter("listing", K::Text { maximum_bytes: 32 })],
        }
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| self.refusal.clone())?;
        actions::offer(
            action,
            &self.action,
            query,
            engine,
            &self.config,
            &self.guard,
            false,
            params,
            &self.refusal,
        )
    }
}

//! Actual listing and quote producers. A listing materializes immutable
//! royalty evidence through Roll; a quote remains unfunded consent data.
use super::codec::{Bounded, Row};
use super::config::ProductChoice;
use super::families::Families;
use super::guard::EntitlementGuard;
use super::records::Cut;
use super::{EntitlementConfig, EntitlementListingV1, EntitlementQuoteV1, EntitlementSubjectV1};
use super::{accounts, actions};
use mush_core::action::{
    ActionError, ActionErrorCode, ActionInvocation, ActionProvider, AuthoritativeActionOutput,
};
use mush_core::{ActionTxn, CommitToken, DefinitionRef, PrepareCtx, RulesValue};
use mush_economy::managed_entitlement::{EntitlementProtocol, Recipient};
use mush_roll::commerce::LicenceId;
use mush_roll::commerce::entitlement::reader::{EntitlementSaleFacts, EntitlementSaleRequest};
use mush_roll::commerce::entitlement::writer::RegisteredEntitlementWriter;
use mush_roll::commerce::paid::PaidForm;
use serde::Deserialize;
#[derive(Clone, Copy)]
pub(super) enum Kind {
    List,
    Quote,
}
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Form {
    Sealed,
    Open,
    OpenResale,
}
impl Form {
    const fn paid(self) -> PaidForm {
        match self {
            Self::Sealed => PaidForm::Sealed {},
            Self::Open => PaidForm::Open {},
            Self::OpenResale => PaidForm::OpenResale {},
        }
    }
}
/// The List request. 17g C2b (D-UM-40, C2 plan §2.5; C1's residual): the
/// author's per-listing transfer mark. An absent `transferable` reads `true`
/// (the wire before the mark), so every existing caller is unchanged; `false`
/// only narrows the deployment policy (`EntitlementConfig::product`). The
/// listing row freezes the product, so no record format moves.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct List {
    owner_licence: LicenceId,
    form: Form,
    royalty_permille: u16,
    #[serde(default = "transferable_unless_marked")]
    transferable: bool,
}
/// The serde default of `List::transferable`: transferable unless the
/// author marks the listing otherwise (D-UM-40).
const fn transferable_unless_marked() -> bool {
    true
}
impl List {
    /// The author's product choice, mark included, exactly as the List
    /// branch hands it to `EntitlementConfig::product` (17g C2b).
    pub(super) const fn choice(&self) -> ProductChoice {
        ProductChoice {
            form: self.form.paid(),
            royalty_permille: self.royalty_permille,
            transferable: self.transferable,
        }
    }
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
    pub protocol: EntitlementProtocol,
    pub kind: Kind,
    pub config: EntitlementConfig,
    pub families: Families,
    pub writer: Option<RegisteredEntitlementWriter>,
    pub guard: EntitlementGuard,
    pub refusal: ActionError,
}
pub(super) fn refusal() -> Result<ActionError, mush_core::ModError> {
    ActionError::new(
        ActionErrorCode::ProviderRejected,
        false,
        "No personal mind entitlement is available under these terms.",
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
        .and_then(|r| {
            if let Request::List(v) = &r
                && (v.royalty_permille > 300
                    || (matches!(v.form, Form::Sealed) && v.royalty_permille != 0))
            {
                return Err("invalid explicit downstream consent".into());
            }
            Ok(r)
        })
    }
    fn prepare(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, String> {
        let commit = ctx.commit_token();
        let kind = match self.request(inv)? {
            Request::List(request) => {
                let writer = self.writer.as_ref().ok_or("listing writer unavailable")?;
                let observed = writer
                    .observe_listing(ctx, txn, request.owner_licence)
                    .map_err(|e| e.to_string())?;
                let facts = observed.facts();
                if facts.requester() != inv.actor()
                    || facts.seller() != inv.actor()
                    || facts.protocol() != self.protocol
                {
                    return Err("listing subject differs".into());
                }
                let world = ctx.world().map_err(|e| e.to_string())?;
                self.guard.current(world, inv.actor())?;
                let economy = accounts::live(world)?;
                accounts::personal(economy, &self.config.policy.instrument, inv.actor())?;
                accounts::cut(
                    economy,
                    &self.config.policy.instrument,
                    &self.config.policy.registry,
                )?;
                accounts::cut(
                    economy,
                    &self.config.policy.instrument,
                    &self.config.policy.venue,
                )?;
                let row = EntitlementListingV1 {
                    format: 1,
                    publication: commit,
                    subject: EntitlementSubjectV1::capture(facts),
                    product: self.config.product(facts, request.choice())?,
                    royalty: facts.royalty().clone(),
                    price: self.config.policy.price,
                    instrument: self.config.policy.instrument.clone(),
                    venue: self.config.policy.venue(),
                    registry: Cut(self.config.policy.registry.clone()),
                    policy: self.config.policy_ref().map_err(|e| e.to_string())?,
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
                        EntitlementSaleRequest::listed(
                            listing.subject.seller,
                            listing.subject.owner_licence,
                        ),
                    )
                    .map_err(|e| e.to_string())?;
                current_listing(&self.config, self.protocol, &listing, &facts, inv.actor())?;
                let pair = ctx
                    .distinct_personal_accounts(listing.subject.seller)
                    .map_err(|e| e.to_string())?;
                if pair.requester() != inv.actor() || pair.other() != listing.subject.seller {
                    return Err("buyer account binding differs".into());
                }
                let world = ctx.world().map_err(|e| e.to_string())?;
                self.guard.current(world, inv.actor())?;
                let economy = accounts::live(world)?;
                let payer = accounts::personal(economy, &listing.instrument, inv.actor())?;
                if payer.approval.is_some() {
                    return Err("direct personal spending account required".into());
                }
                let seller =
                    accounts::personal(economy, &listing.instrument, listing.subject.seller)?;
                accounts::cut(economy, &listing.instrument, &listing.registry.0)?;
                accounts::cut(economy, &listing.instrument, &listing.venue.cut.0)?;
                let parents =
                    accounts::parents(world, economy, &listing.instrument, &listing.royalty)?;
                let mut q = EntitlementQuoteV1 {
                    format: 1,
                    publication: commit,
                    listing: request.listing,
                    listing_digest: listing.digest()?,
                    buyer: inv.actor(),
                    subject: listing.subject.clone(),
                    product_digest: listing.product.digest()?,
                    product: listing.product.clone(),
                    royalty: listing.royalty.clone(),
                    price: listing.price,
                    instrument: listing.instrument.clone(),
                    registry: listing.registry.clone(),
                    venue: listing.venue.clone(),
                    parents,
                    remainder: Recipient {
                        account: seller.id,
                        owner: mush_org::PrincipalId::Object(listing.subject.seller),
                    },
                    payer: payer.id,
                    payouts: Bounded(vec![]),
                    policy: listing.policy.clone(),
                    quoted_at_utc: world.now(),
                    expires_at_utc: world
                        .now()
                        .checked_add(self.config.policy.quote_seconds)
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
                self.config.policy_ref().map_err(|e| e.to_string())?,
            ])
            .with_eligible_witnesses(vec![inv.principal().clone()]))
    }
}
pub(super) fn current_listing(
    config: &EntitlementConfig,
    protocol: EntitlementProtocol,
    l: &EntitlementListingV1,
    f: &EntitlementSaleFacts,
    actor: mush_core::ObjId,
) -> Result<(), String> {
    l.validate()?;
    // 17g C2b: recompute with the listing's own frozen choice, mark included
    // (`ProductChoice::frozen`). A marked listing stays current; an unmarked
    // one still lapses if the policy turns false (`false && true` differs
    // from its frozen `true`), as before the mark.
    let expected = config.product(f, ProductChoice::frozen(&l.product))?;
    if f.requester() != actor
        || f.protocol() != protocol
        || l.subject != EntitlementSubjectV1::capture(f)
        || l.product != expected
        || &l.royalty != f.royalty()
        || current_policy(config, l).is_err()
    {
        return Err("listing no longer matches actual current entitlement".into());
    }
    Ok(())
}
/// The policy half of `current_listing`: the listing's price, instrument,
/// Registry and venue cuts and policy reference are the deployment's current
/// ones. 17g C2c: `mind_commands` lists exactly the listings that pass this
/// test (the C2c brief: "`current_listing`'s policy test only — not
/// seller-specific"); the Quote branch still runs the whole of
/// `current_listing` against Roll's current sale facts, so a listing this
/// shows can still refuse a quote (a withdrawn revision, a lapsed owner).
pub(crate) fn current_policy(
    config: &EntitlementConfig,
    l: &EntitlementListingV1,
) -> Result<(), String> {
    l.validate()?;
    if l.price != config.policy.price
        || l.instrument != config.policy.instrument
        || l.registry.0 != config.policy.registry
        || l.venue != config.policy.venue()
        || l.policy != config.policy_ref().map_err(|e| e.to_string())?
    {
        return Err("listing no longer matches the current entitlement policy".into());
    }
    Ok(())
}
/// The projected parameters of the List and Quote offers. Core refuses any
/// supplied key the schema does not name, so the List schema is what admits
/// the author's mark at all. 17g C2b (D-UM-40): `transferable` is optional;
/// absent is transferable (`List`'s serde default).
pub(super) fn parameters(kind: Kind) -> Result<Vec<mush_core::ProjectedParameter>, String> {
    use mush_core::ProjectedParameterKind as K;
    match kind {
        Kind::List => vec![
            actions::parameter("owner_licence", K::Text { maximum_bytes: 20 }),
            actions::parameter(
                "form",
                K::Choice {
                    choices: vec!["sealed".into(), "open".into(), "open_resale".into()],
                },
            ),
            actions::parameter(
                "royalty_permille",
                K::Integer {
                    minimum: 0,
                    maximum: 300,
                },
            ),
            actions::optional("transferable", K::Boolean),
        ],
        Kind::Quote => vec![actions::parameter("listing", K::Text { maximum_bytes: 32 })],
    }
    .into_iter()
    .collect()
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
        let params = parameters(self.kind).map_err(|_| self.refusal.clone())?;
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

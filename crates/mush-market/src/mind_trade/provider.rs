//! Real typed offer providers. Quote preparation observes a product; it does
//! not reserve funding capacity or authorize the eventual managed E4 effect.
use super::records::*;
use super::{Families, OriginalQuoteConfig};
use mush_core::action::{
    ActionError, ActionErrorCode, ActionInvocation, ActionProvider, AuthoritativeActionOutput,
    PerceivedRef, PrincipalId,
};
use mush_core::action_state::ActionTxn;
use mush_core::read_service::OwnerReadHandle;
use mush_core::{CommitToken, DefinitionRef, ObjId, PrepareCtx, RulesValue, World};
use mush_economy::{Account, AccountId, AccountStatus, EconomyStore, InstrumentStatus};
use mush_roll::commerce::LicenceId;
use mush_roll::commerce::paid::PaidForm;
use mush_roll::commerce::proceeds::{GrossCut, calculate};
use mush_roll::commerce::records::RevisionPolicy;
use mush_roll::commerce::sale_reader::{OriginalSaleReader, OriginalSaleRequest};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

#[derive(Clone, Copy)]
pub(super) enum Kind {
    List,
    Quote,
    RetireListing,
    RetireQuote,
    Expire,
}
pub(super) struct Provider {
    action: DefinitionRef,
    outcome: DefinitionRef,
    kind: Kind,
    config: OriginalQuoteConfig,
    families: Families,
    reader: OwnerReadHandle<OriginalSaleReader>,
    gate: Arc<dyn crate::VenueGate>,
    refusal: ActionError,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListRequest {
    owner_licence: LicenceId,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceFormsListRequest {
    owner_licence: LicenceId,
    form: super::protocol::SourceForm,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QuoteRequest {
    listing: CommitToken,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RetireRequest {
    publication: CommitToken,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpireRequest {
    quotes: Vec<CommitToken>,
}
enum Request {
    List(LicenceId, PaidForm),
    Quote(QuoteRequest),
    RetireListing(RetireRequest),
    RetireQuote(RetireRequest),
    Expire(ExpireRequest),
}
impl Provider {
    pub(super) fn new(
        action: DefinitionRef,
        outcome: DefinitionRef,
        kind: Kind,
        config: OriginalQuoteConfig,
        families: Families,
        reader: OwnerReadHandle<OriginalSaleReader>,
        gate: Arc<dyn crate::VenueGate>,
    ) -> Result<Self, mush_core::ModError> {
        let refusal = ActionError::new(
            ActionErrorCode::ProviderRejected,
            false,
            "No original personal mind offer is available under these terms.",
        )
        .map_err(super::error)?;
        Ok(Self {
            action,
            outcome,
            kind,
            config,
            families,
            reader,
            gate,
            refusal,
        })
    }
    fn request(&self, inv: &ActionInvocation) -> Result<Request, String> {
        if inv.principal() != &(PrincipalId::Object { id: inv.actor() })
            || inv.subject() != &PerceivedRef::SelfActor
            || !inv.targets().is_empty()
            || inv.idempotency().scope().owner != self.action.key().owner
        {
            return Err("invalid original quote requester".into());
        }
        match self.kind {
            Kind::List if self.config.source_forms => {
                serde_json::from_value::<SourceFormsListRequest>(inv.parameters().clone())
                    .map(|r| Request::List(r.owner_licence, r.form.paid()))
            }
            Kind::List => serde_json::from_value::<ListRequest>(inv.parameters().clone())
                .map(|r| Request::List(r.owner_licence, PaidForm::Sealed {})),
            Kind::Quote => serde_json::from_value(inv.parameters().clone()).map(Request::Quote),
            Kind::RetireListing => {
                serde_json::from_value(inv.parameters().clone()).map(Request::RetireListing)
            }
            Kind::RetireQuote => {
                serde_json::from_value(inv.parameters().clone()).map(Request::RetireQuote)
            }
            Kind::Expire => serde_json::from_value(inv.parameters().clone()).map(Request::Expire),
        }
        .map_err(|e| e.to_string())
        .and_then(|request| {
            if let Request::Expire(row) = &request {
                if inv.actor() != mush_core::GOD
                    || row.quotes.is_empty()
                    || row.quotes.len() > super::index::RETIRE_BATCH
                    || row
                        .quotes
                        .iter()
                        .copied()
                        .collect::<std::collections::BTreeSet<_>>()
                        .len()
                        != row.quotes.len()
                {
                    return Err("invalid expiry actor or bounded batch".into());
                }
            }
            Ok(request)
        })
    }
    fn venue(&self, world: &World, actor: ObjId) -> Result<(), String> {
        let venue = crate::MarketId::new(&self.config.policy.venue.name).ok_or("invalid venue")?;
        let room = world.containing_room(actor).ok_or("no current venue")?;
        if self.gate.venue_of_room(world, room).as_ref() != Some(&venue) {
            return Err("actor is not at the configured original-mind venue".into());
        }
        self.gate
            .admit(world, actor, &venue, None, crate::Presence::Here)
            .map_err(|_| "venue refuses original offer".into())
    }
    fn prepare(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, String> {
        let commit = ctx.commit_token();
        let request = self.request(inv)?;
        let policy = &self.config.policy;
        let policy_digest = self.config.policy_digest()?;
        let word = match request {
            Request::List(owner_licence, form) => {
                let facts = ctx
                    .read_owner_preimage(txn, &self.reader, OriginalSaleRequest::own(owner_licence))
                    .map_err(|e| e.to_string())?;
                if facts.requester() != inv.actor() || facts.seller() != inv.actor() {
                    return Err("original listing requester differs".into());
                }
                let world = ctx.world().map_err(|e| e.to_string())?;
                self.venue(world, inv.actor())?;
                let economy = live_economy(world)?;
                personal_account(economy, &policy.instrument, inv.actor())?;
                recipients(economy, policy)?;
                let subject = OriginalSubject::capture(&facts);
                let product = OriginalProduct {
                    kind: if self.config.source_forms {
                        ProductKind::OriginalPersonalSourceForms {}
                    } else {
                        ProductKind::OriginalPersonal {}
                    },
                    class: facts.minimum(),
                    seats: 1,
                    form,
                    revision: RevisionPolicy::Pin {
                        filed: facts.filed(),
                        revision: facts.revision(),
                    },
                    transferable: policy.transferable,
                    term_seconds: policy.term_seconds,
                    grace_seconds: policy.grace_seconds,
                };
                let listing = MindListing {
                    format: self.config.protocol().format(),
                    publication: commit,
                    subject,
                    product,
                    price: policy.price,
                    instrument: policy.instrument.clone(),
                    venue: policy.venue.clone(),
                    registry_recipient: policy.registry_recipient.clone(),
                    registry_account: policy.registry_account,
                    registry_permille: policy.registry_permille,
                    policy: policy_digest,
                    observed_at_utc: world.now(),
                };
                listing.validate()?;
                super::index::publish_listing(txn, &self.families, listing)?;
                "listing"
            }
            Request::Quote(request) => {
                // These providers never emit same-commit triggers or replace a
                // listing. A newly inserted listing has this very commit; it
                // cannot be quoted before its own acknowledgement.
                if request.listing == commit {
                    return Err("listing is not yet acknowledged".into());
                }
                let listing = txn
                    .get(&self.families.listings, &request.listing)
                    .map_err(|e| e.to_string())?
                    .cloned()
                    .ok_or("no original listing")?;
                listing.validate()?;
                if listing.publication != request.listing
                    || listing.policy != policy_digest
                    || listing.product.protocol() != self.config.protocol()
                {
                    return Err("original listing identity or configured policy changed".into());
                }
                let facts = ctx
                    .read_owner_preimage(
                        txn,
                        &self.reader,
                        OriginalSaleRequest::listed(
                            listing.subject.seller,
                            listing.subject.owner_licence,
                        ),
                    )
                    .map_err(|e| e.to_string())?;
                if facts.requester() != inv.actor()
                    || OriginalSubject::capture(&facts) != listing.subject
                {
                    return Err("original listing subject has changed".into());
                }
                let expected_product = OriginalProduct {
                    kind: if self.config.source_forms {
                        ProductKind::OriginalPersonalSourceForms {}
                    } else {
                        ProductKind::OriginalPersonal {}
                    },
                    class: facts.minimum(),
                    seats: 1,
                    form: listing.product.form,
                    revision: RevisionPolicy::Pin {
                        filed: facts.filed(),
                        revision: facts.revision(),
                    },
                    transferable: policy.transferable,
                    term_seconds: policy.term_seconds,
                    grace_seconds: policy.grace_seconds,
                };
                if listing.product != expected_product
                    || listing.price != policy.price
                    || listing.instrument != policy.instrument
                    || listing.venue != policy.venue
                    || listing.registry_recipient != policy.registry_recipient
                    || listing.registry_account != policy.registry_account
                    || listing.registry_permille != policy.registry_permille
                {
                    return Err("listing terms differ from the actual configured product".into());
                }
                // This is the actual Engine account oracle observation. It
                // retains map+binding epochs through installation and exposes
                // no authentication account ids to this row or its journal.
                let pair = ctx
                    .distinct_personal_accounts(listing.subject.seller)
                    .map_err(|e| e.to_string())?;
                if pair.requester() != inv.actor() || pair.other() != listing.subject.seller {
                    return Err("account observation identity differs".into());
                }
                let world = ctx.world().map_err(|e| e.to_string())?;
                self.venue(world, inv.actor())?;
                let economy = live_economy(world)?;
                let payer = personal_account(economy, &policy.instrument, inv.actor())?;
                let seller = personal_account(economy, &policy.instrument, listing.subject.seller)?;
                recipients(economy, policy)?;
                if payer.approval.is_some() {
                    return Err("original quote requires a direct personal spending account".into());
                }
                let payouts = payouts(
                    calculate(
                        policy.price,
                        GrossCut {
                            to: policy.registry_account,
                            permille: policy.registry_permille,
                        },
                        Some(GrossCut {
                            to: policy.venue.account,
                            permille: policy.venue.permille,
                        }),
                        &[],
                        seller.id,
                    )
                    .map_err(|e| e.to_string())?,
                )?;
                let now = world.now();
                let quote = MindQuote {
                    format: self.config.protocol().format(),
                    publication: commit,
                    listing: listing.publication,
                    listing_digest: listing.digest()?,
                    buyer: inv.actor(),
                    subject: listing.subject.clone(),
                    product: listing.product.clone(),
                    product_digest: listing.product.digest()?,
                    price: policy.price,
                    instrument: policy.instrument.clone(),
                    venue: policy.venue.clone(),
                    registry_recipient: policy.registry_recipient.clone(),
                    registry_account: policy.registry_account,
                    registry_permille: policy.registry_permille,
                    payer: payer.id,
                    seller_account: seller.id,
                    payouts,
                    policy: policy_digest,
                    quoted_at_utc: now,
                    expires_at_utc: now
                        .checked_add(policy.quote_seconds)
                        .ok_or("quote expiry overflow")?,
                };
                quote.validate()?;
                super::index::publish_quote(txn, &self.families, quote)?;
                "quote"
            }
            Request::RetireListing(request) => {
                super::index::retire_listing(
                    txn,
                    &self.families,
                    request.publication,
                    inv.actor(),
                )?;
                "listing_retired"
            }
            Request::RetireQuote(request) => {
                let now = ctx.world().map_err(|e| e.to_string())?.now();
                super::index::retire_quotes(
                    txn,
                    &self.families,
                    &[request.publication],
                    Some(inv.actor()),
                    now,
                )?;
                "quote_retired"
            }
            Request::Expire(request) => {
                let now = ctx.world().map_err(|e| e.to_string())?.now();
                super::index::retire_quotes(txn, &self.families, &request.quotes, None, now)?;
                "quotes_expired"
            }
        };
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({"kind":word,"publication":commit}),
        )
        .map_err(|e| e.to_string())?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_definitions(vec![self.outcome.clone(), self.config.manifest.clone()])
            .with_eligible_witnesses(vec![inv.principal().clone()]))
    }
}
fn live_economy(world: &World) -> Result<&EconomyStore, String> {
    let store = world
        .module_state::<EconomyStore>()
        .ok_or("economy unavailable")?;
    if world
        .module_state_owner(std::any::TypeId::of::<EconomyStore>())
        .is_none_or(|owner| owner.as_str() != "economy")
        || store.is_frozen()
    {
        return Err("economy unavailable".into());
    }
    Ok(store)
}
fn personal_account<'a>(
    economy: &'a EconomyStore,
    instrument: &mush_economy::InstrumentId,
    actor: ObjId,
) -> Result<&'a Account, String> {
    let snapshot = economy.snapshot();
    if snapshot
        .instrument(instrument)
        .is_none_or(|i| i.status != InstrumentStatus::Active)
    {
        return Err("quote instrument unavailable".into());
    }
    snapshot
        .accounts_of(&mush_org::PrincipalId::Object(actor))
        .find(|a| {
            a.instrument == *instrument && a.scope.is_none() && a.status == AccountStatus::Active
        })
        .ok_or_else(|| "personal quote account unavailable".into())
}
fn recipient(
    economy: &EconomyStore,
    id: AccountId,
    owner: &mush_org::PrincipalId,
    instrument: &mush_economy::InstrumentId,
) -> Result<(), String> {
    if economy.snapshot().account(id).is_none_or(|a| {
        a.owner != *owner || a.instrument != *instrument || a.status != AccountStatus::Active
    }) {
        return Err("configured quote recipient unavailable".into());
    }
    Ok(())
}
fn recipients(economy: &EconomyStore, p: &super::OriginalQuotePolicy) -> Result<(), String> {
    recipient(
        economy,
        p.registry_account,
        &mush_org::PrincipalId::System(p.registry_recipient.clone()),
        &p.instrument,
    )?;
    recipient(economy, p.venue.account, &p.venue.recipient, &p.instrument)
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
        if action != &self.action
            || query.truth_subject() != &(mush_core::SubjectKey::Object { id: query.actor() })
            || query.projected_subject() != &PerceivedRef::SelfActor
            || engine.world.get(query.actor()).is_none_or(|o| {
                o.kind != mush_core::ObjKind::Player || o.flags.contains(mush_core::Flags::GOING)
            })
            || (matches!(self.kind, Kind::List | Kind::Quote)
                && self.venue(&engine.world, query.actor()).is_err())
            || (matches!(self.kind, Kind::Expire) && query.actor() != mush_core::GOD)
        {
            return Ok(None);
        }
        let bad = || self.refusal.clone();
        let defs = engine.registry.definitions().ok_or_else(bad)?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(mush_core::RevisionSet::default())
            .map_err(|_| bad())?;
        guards
            .add_definition(self.action.clone(), defs)
            .map_err(|_| bad())?;
        guards
            .add_definition(self.config.manifest.clone(), defs)
            .map_err(|_| bad())?;
        let field = match self.kind {
            Kind::List => Some(("owner_licence", 20)),
            Kind::Quote => Some(("listing", 32)),
            Kind::RetireListing | Kind::RetireQuote => Some(("publication", 32)),
            Kind::Expire => None,
        };
        let fixed_parameters = if matches!(self.kind, Kind::Expire) {
            let page = engine
                .action_state()
                .scan_family(
                    &self.families.indices.expiry,
                    None,
                    super::index::RETIRE_BATCH,
                )
                .map_err(|_| bad())?;
            let mut quotes = Vec::new();
            for entry in page.entries() {
                let row = entry.value();
                if row.key().encoded() != entry.encoded_key() {
                    return Err(bad());
                }
                if row.at > engine.world.now() {
                    break;
                }
                quotes.push(row.quote);
            }
            if quotes.is_empty() {
                return Ok(None);
            }
            json!({"quotes":quotes})
        } else {
            json!({})
        };
        let mut parameters: Vec<_> = field
            .map(|(field, bytes)| -> Result<_, ActionError> {
                Ok(mush_core::ProjectedParameter {
                    id: mush_core::mechanics::CanonicalId::new(field).map_err(|_| bad())?,
                    required: true,
                    kind: mush_core::ProjectedParameterKind::Text {
                        maximum_bytes: bytes,
                    },
                })
            })
            .transpose()?
            .into_iter()
            .collect();
        if matches!(self.kind, Kind::List) && self.config.source_forms {
            parameters.push(mush_core::ProjectedParameter {
                id: mush_core::mechanics::CanonicalId::new("form").map_err(|_| bad())?,
                required: true,
                kind: mush_core::ProjectedParameterKind::Choice {
                    choices: vec!["sealed".into(), "open".into(), "open_resale".into()],
                },
            });
        }
        Ok(Some(mush_core::ProviderOfferDraft {
            fixed_parameters,
            parameters: mush_core::ProjectedParameterSchema::new(parameters).map_err(|_| bad())?,
            targets: mush_core::ProjectedTargetSchema::None,
            required_channels: vec![],
            expires: None,
            presentation: mush_core::ActionPresentation::new(
                "market-original-mind-offer",
                "Record an original mind offer",
                "Record listing or quote terms; no payment or licence is created.",
            )
            .map_err(|_| bad())?,
            guards: guards.finish().map_err(|_| bad())?,
        }))
    }
}

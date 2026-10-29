//! Exact immutable consent data, separate from the legacy original products.
use super::codec::{self, Bounded, Row};
use mush_core::{CommitToken, DefinitionDigest, DefinitionRef, ObjId};
use mush_economy::Holder;
use mush_economy::managed_entitlement::{
    EntitlementProtocol, EntitlementPurchase, GrossRecipientCut, Recipient, RoyaltyRecipientCut,
    RoyaltyTitleRecipient,
};
use mush_economy::{AccountId, Amount, InstrumentId};
use mush_roll::commerce::entitlement::{FiledRoyaltyTermsV1, GrantEconomicsV1};
use mush_roll::commerce::filed::ParentContent;
use mush_roll::commerce::paid::{PaidForm, PaidTerms};
use mush_roll::commerce::rating::{RatingClass, RatingRef};
use mush_roll::commerce::records::RevisionPolicy;
use mush_roll::commerce::{
    CharterId, FiledRevisionId, LicenceId, LineId, MAX_GRACE_SECONDS, MAX_TERM_SECONDS,
    RevisionIdentity, TermGeneration, TermWindow,
};
use serde::{Deserialize, Deserializer, Serialize};

pub const MAX_LISTING_BYTES: usize = 16384;
pub const MAX_QUOTE_BYTES: usize = 24576;
fn invalid() -> String {
    "entitlement consent records disagree".into()
}
fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}

macro_rules! data {
    ($name:ident,$wire:ident,{$($(#[$attr:meta])* $field:ident:$ty:ty),* $(,)?}) => {
        #[derive(Clone,Debug,PartialEq,Eq,Serialize)]
        pub struct $name {$(pub(super) $field:$ty),*}
        #[derive(Deserialize)] #[serde(deny_unknown_fields)]
        struct $wire {$($(#[$attr])* $field:$ty),*}
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D:Deserializer<'de>>(d:D)->Result<Self,D::Error> {
                let row=$wire::deserialize(d)?; let value=Self {$($field:row.$field),*};
                value.validate().map_err(serde::de::Error::custom)?; Ok(value)
            }
        }
    };
}
data!(EntitlementSubjectV1,SubjectWire,{
    seller:ObjId,title_revision:u64,charter:CharterId,line:LineId,filed:FiledRevisionId,sequence:u64,
    owner_licence:LicenceId,owner_term:TermGeneration,owner_window:TermWindow,
    revision:RevisionIdentity,schema:DefinitionDigest,definition:DefinitionDigest,rating:RatingRef,
    compatibility:DefinitionDigest,declared:RatingClass,minimum:RatingClass,
    #[serde(deserialize_with="codec::required")] parent:Option<ParentContent>,
});
impl EntitlementSubjectV1 {
    pub(super) fn validate(&self) -> Result<(), String> {
        if self.seller.0 == 0
            || self.title_revision == 0
            || self.sequence == 0
            || self.minimum < self.declared
            || self.parent.is_some_and(|p| {
                p.filed == self.filed || p.revision.revision().key == self.revision.revision().key
            })
        {
            return Err(invalid());
        }
        Ok(())
    }
    pub const fn seller(&self) -> ObjId {
        self.seller
    }
    pub const fn filed(&self) -> FiledRevisionId {
        self.filed
    }
    pub const fn revision(&self) -> RevisionIdentity {
        self.revision
    }
    pub const fn parent(&self) -> Option<ParentContent> {
        self.parent
    }
}
data!(EntitlementProductV1,ProductWire,{
    protocol:EntitlementProtocol,terms:PaidTerms,revision:RevisionPolicy,
    term_seconds:i64,grace_seconds:i64,grant:GrantEconomicsV1,
});
impl EntitlementProductV1 {
    pub(super) fn validate(&self) -> Result<(), String> {
        self.grant.validate().map_err(error)?;
        if self.terms.seats().get() != 1
            || !(1..=MAX_TERM_SECONDS).contains(&self.term_seconds)
            || !(0..=MAX_GRACE_SECONDS).contains(&self.grace_seconds)
            || !matches!(self.revision, RevisionPolicy::Pin { .. })
            || self.terms != self.grant.renewal().terms()
            || self.revision != self.grant.renewal().revision()
        {
            return Err(invalid());
        }
        Ok(())
    }
    pub(super) fn matches_subject(&self, s: &EntitlementSubjectV1) -> Result<(), String> {
        self.validate()?;
        s.validate()?;
        if self.terms.class() < s.minimum
            || self.revision
                != (RevisionPolicy::Pin {
                    filed: s.filed,
                    revision: s.revision,
                })
            || matches!(
                (self.protocol, s.parent),
                (EntitlementProtocol::OriginalV3, Some(_)) | (EntitlementProtocol::DerivedV1, None)
            )
        {
            return Err(invalid());
        }
        Ok(())
    }
    pub fn digest(&self) -> Result<DefinitionDigest, String> {
        self.validate()?;
        codec::digest(
            super::protocol::domain(self.protocol, "product")?,
            self,
            MAX_LISTING_BYTES,
        )
    }
    pub const fn protocol(&self) -> EntitlementProtocol {
        self.protocol
    }
    pub fn grant(&self) -> &GrantEconomicsV1 {
        &self.grant
    }
}

/// Keep E4's exact wire fields while requiring explicit null for a zero cut.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(super) struct Cut(pub(super) GrossRecipientCut);
impl<'de> Deserialize<'de> for Cut {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            permille: u16,
            #[serde(deserialize_with = "codec::required")]
            recipient: Option<Recipient>,
        }
        let row = Wire::deserialize(d)?;
        let value = Self(GrossRecipientCut {
            permille: row.permille,
            recipient: row.recipient,
        });
        value.validate().map_err(serde::de::Error::custom)?;
        Ok(value)
    }
}
impl Cut {
    fn validate(&self) -> Result<(), String> {
        if self.0.permille > 1000 || (self.0.permille == 0) != self.0.recipient.is_none() {
            return Err(invalid());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(super) struct ParentCut(pub(super) RoyaltyRecipientCut);
impl<'de> Deserialize<'de> for ParentCut {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            charter: ObjId,
            permille: u16,
            #[serde(deserialize_with = "codec::required")]
            title: Option<RoyaltyTitleRecipient>,
        }
        let row = Wire::deserialize(d)?;
        if row.charter.0 == 0
            || row.permille > 300
            || (row.permille == 0) != row.title.is_none()
            || row.title.as_ref().is_some_and(|t| {
                t.title_revision == 0
                    || !matches!(t.recipient.owner,mush_org::PrincipalId::Object(id) if id.0!=0)
            })
        {
            return Err(serde::de::Error::custom(invalid()));
        }
        Ok(Self(RoyaltyRecipientCut {
            charter: row.charter,
            permille: row.permille,
            title: row.title,
        }))
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Venue {
    pub(super) name: crate::MarketId,
    pub(super) cut: Cut,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Payout {
    pub(super) account: AccountId,
    pub(super) owner: mush_org::PrincipalId,
    pub(super) amount: Amount,
}

#[allow(clippy::too_many_arguments)]
fn common(
    subject: &EntitlementSubjectV1,
    product: &EntitlementProductV1,
    royalty: &FiledRoyaltyTermsV1,
    price: Amount,
    instrument: &InstrumentId,
    policy: &DefinitionRef,
    registry: &Cut,
    venue: &Venue,
) -> Result<(), String> {
    product.matches_subject(subject)?;
    let reference = royalty.reference().map_err(error)?;
    if royalty.filed() != subject.filed
        || royalty.revision() != subject.revision
        || product.grant.royalty_chain() != reference
        || royalty.edges().first().map(|e| e.parent()) != subject.parent
        || (subject.parent.is_none() && !royalty.edges().is_empty())
        || price.is_negative()
        || price.is_zero()
    {
        return Err(invalid());
    }
    product
        .grant
        .renewal()
        .matches_product(product.terms, product.revision, instrument, policy)
        .map_err(error)?;
    if product.terms.form() == (PaidForm::OpenResale {}) {
        royalty
            .check_next_resale(
                product
                    .grant
                    .downstream()
                    .royalty_permille()
                    .ok_or_else(invalid)?,
            )
            .map_err(error)?;
    }
    registry.validate()?;
    venue.cut.validate()?;
    let sum = u32::from(registry.0.permille)
        + u32::from(venue.cut.0.permille)
        + u32::from(royalty.total_permille().map_err(error)?);
    if sum > 1000 {
        return Err(invalid());
    }
    Ok(())
}
data!(EntitlementListingV1,ListingWire,{
    format:u16,publication:CommitToken,subject:EntitlementSubjectV1,product:EntitlementProductV1,
    royalty:FiledRoyaltyTermsV1,price:Amount,instrument:InstrumentId,venue:Venue,registry:Cut,
    policy:DefinitionRef,observed_at_utc:i64,
});
impl EntitlementListingV1 {
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        codec::decode(bytes)
    }
    pub fn digest(&self) -> Result<DefinitionDigest, String> {
        self.validate()?;
        codec::digest(
            super::protocol::domain(self.product.protocol, "listing")?,
            self,
            Self::MAX_BYTES,
        )
    }
    pub const fn publication(&self) -> CommitToken {
        self.publication
    }
    /// 17g C2c (C2 plan §2.8): what `+minds` may show of this listing, every
    /// field from the frozen row: the protocol, the charter (a public Roll
    /// entry), the product's class, form, transfer mark, term and Grace, the
    /// price, the Registry and venue cuts and the parents' total royalty. No
    /// seller, author, account, revision, rating or digest leaves here.
    pub(crate) fn command_view(&self) -> Result<crate::mind_commands::PublicListingView, String> {
        self.validate()?;
        Ok(crate::mind_commands::PublicListingView {
            publication: self.publication,
            protocol: crate::mind_commands::Protocol::of(self.product.protocol),
            charter: self.subject.charter,
            class: self.product.terms.class(),
            form: crate::mind_commands::ListingForm::of(self.product.terms.form()),
            transferable: self.product.terms.transferable(),
            term_seconds: self.product.term_seconds,
            grace_seconds: self.product.grace_seconds,
            price_minor: self.price.minor(),
            levy_permille: self.registry.0.permille,
            venue_permille: self.venue.cut.0.permille,
            parent_royalty_permille: self.royalty.total_permille().map_err(error)?,
            derived: self.subject.parent.is_some(),
        })
    }
}
impl Row for EntitlementListingV1 {
    const FAMILY: &'static str = "market_mind_entitlement_listing";
    const MAX_BYTES: usize = MAX_LISTING_BYTES;
    fn key(&self) -> String {
        self.publication.to_hex()
    }
    fn validate(&self) -> Result<(), String> {
        if self.format != 1 {
            return Err(invalid());
        }
        common(
            &self.subject,
            &self.product,
            &self.royalty,
            self.price,
            &self.instrument,
            &self.policy,
            &self.registry,
            &self.venue,
        )?;
        codec::bounded(self, Self::MAX_BYTES)?;
        Ok(())
    }
}
data!(EntitlementQuoteV1,QuoteWire,{
    format:u16,publication:CommitToken,listing:CommitToken,listing_digest:DefinitionDigest,
    buyer:Holder,subject:EntitlementSubjectV1,product:EntitlementProductV1,product_digest:DefinitionDigest,
    royalty:FiledRoyaltyTermsV1,price:Amount,instrument:InstrumentId,registry:Cut,venue:Venue,
    parents:Bounded<ParentCut,3>,remainder:Recipient,payer:AccountId,payouts:Bounded<Payout,6>,
    policy:DefinitionRef,quoted_at_utc:i64,expires_at_utc:i64,
});
impl EntitlementQuoteV1 {
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        codec::decode(bytes)
    }
    pub(super) fn raw_digest(&self) -> Result<DefinitionDigest, String> {
        codec::digest(
            super::protocol::domain(self.product.protocol, "quote")?,
            self,
            Self::MAX_BYTES,
        )
    }
    pub fn digest(&self) -> Result<DefinitionDigest, String> {
        self.validate()?;
        self.raw_digest()
    }
    pub const fn publication(&self) -> CommitToken {
        self.publication
    }
    /// 17g C2g: the listing this quote prices (its publication token), which
    /// `mind_commands`' own-purchase read matches a typed code against.
    pub(crate) const fn listing(&self) -> CommitToken {
        self.listing
    }
    /// 17g C2g: the quoting buyer (M8c B1: a character, or an organisation
    /// through the quoting character).
    pub(crate) const fn buyer(&self) -> Holder {
        self.buyer
    }
    /// 17g C2g: the frozen price, in minor units.
    pub(crate) const fn price_minor(&self) -> i64 {
        self.price.minor()
    }
    /// 17g C2g: when the quote was made, world UTC seconds.
    pub(crate) const fn quoted_at_utc(&self) -> i64 {
        self.quoted_at_utc
    }
    /// 17g C2g: when the quote stops being fundable, world UTC seconds
    /// (`provider::current` refuses a Fund at or after it).
    pub(crate) const fn expires_at_utc(&self) -> i64 {
        self.expires_at_utc
    }
    /// 17g C2g: the entitlement protocol of the quote's product.
    pub(crate) const fn protocol(&self) -> EntitlementProtocol {
        self.product.protocol
    }
    pub(super) fn economic_data(
        &self,
        quote_digest: DefinitionDigest,
    ) -> Result<EntitlementPurchase, String> {
        Ok(EntitlementPurchase {
            protocol: self.product.protocol,
            quote: self.publication,
            quote_digest,
            product_digest: self.product_digest,
            buyer: self.buyer,
            seller: self.subject.seller,
            charter: ObjId(self.subject.charter.get()),
            title_revision: self.subject.title_revision,
            payer: self.payer,
            instrument: self.instrument.clone(),
            price: self.price,
            registry: self.registry.0.clone(),
            venue: Some(self.venue.cut.0.clone()),
            parents: self.parents.0.iter().map(|p| p.0.clone()).collect(),
            remainder: self.remainder.clone(),
            royalty_chain_digest: self.royalty.digest().map_err(error)?,
            grant_economics_digest: self.product.grant.digest().map_err(error)?,
            quoted_at_utc: self.quoted_at_utc,
            expires_at_utc: self.expires_at_utc,
        })
    }
    /// Pure E4 input, not an E4 result or account/current-rights capability.
    pub(super) fn economic_subject(&self) -> Result<EntitlementPurchase, String> {
        self.validate()?;
        self.economic_data(self.raw_digest()?)
    }
}
impl Row for EntitlementQuoteV1 {
    const FAMILY: &'static str = "market_mind_entitlement_quote";
    const MAX_BYTES: usize = MAX_QUOTE_BYTES;
    fn key(&self) -> String {
        self.publication.to_hex()
    }
    fn validate(&self) -> Result<(), String> {
        if self.format != 1
            || self.publication == self.listing
            || self.product_digest != self.product.digest()?
            || self.parents.0.len() != self.royalty.edges().len()
            || self.parents.0.len() > 3
            || self.payouts.0.is_empty()
            || self.payouts.0.len() > 6
        {
            return Err(invalid());
        }
        common(
            &self.subject,
            &self.product,
            &self.royalty,
            self.price,
            &self.instrument,
            &self.policy,
            &self.registry,
            &self.venue,
        )?;
        for (cut, edge) in self.parents.0.iter().zip(self.royalty.edges()) {
            if cut.0.charter != ObjId(edge.charter().get())
                || cut.0.permille != edge.royalty_permille()
            {
                return Err(invalid());
            }
        }
        let expected = self
            .economic_data(self.raw_digest()?)?
            .payouts()
            .map_err(error)?;
        if expected.len() != self.payouts.0.len()
            || expected
                .iter()
                .zip(&self.payouts.0)
                .any(|(a, b)| a.account != b.account || a.owner != b.owner || a.amount != b.amount)
        {
            return Err(invalid());
        }
        codec::bounded(self, Self::MAX_BYTES)?;
        Ok(())
    }
}

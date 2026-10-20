//! Exact immutable consent data, separate from the legacy original products.
use super::codec::{self, Bounded, Row};
use mush_core::{CommitToken, DefinitionDigest, DefinitionRef, ObjId};
use mush_economy::managed_entitlement::{GrossRecipientCut, Recipient};
use mush_economy::managed_house_purchase::HousePurchase;
use mush_economy::{AccountId, Amount, InstrumentId};
use mush_roll::commerce::house_catalogue::HouseIssuer;
use mush_roll::commerce::house_purchase::{HouseGrantEconomicsV1, HouseProtocol};
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
    "house consent records disagree".into()
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
data!(HouseSubjectV1,SubjectWire,{
    issuer:HouseIssuer,title_revision:u64,charter:CharterId,line:LineId,filed:FiledRevisionId,sequence:u64,
    owner_licence:LicenceId,owner_term:TermGeneration,owner_window:TermWindow,
    revision:RevisionIdentity,schema:DefinitionDigest,definition:DefinitionDigest,rating:RatingRef,
    compatibility:DefinitionDigest,declared:RatingClass,minimum:RatingClass,
});
impl HouseSubjectV1 {
    pub(super) fn validate(&self) -> Result<(), String> {
        if self.title_revision == 0 || self.sequence == 0 || self.minimum < self.declared {
            return Err(invalid());
        }
        Ok(())
    }
    pub const fn issuer(&self) -> HouseIssuer {
        self.issuer
    }
    pub const fn filed(&self) -> FiledRevisionId {
        self.filed
    }
    pub const fn revision(&self) -> RevisionIdentity {
        self.revision
    }
}
data!(HouseProductV1,ProductWire,{
    protocol:HouseProtocol,terms:PaidTerms,revision:RevisionPolicy,
    term_seconds:i64,grace_seconds:i64,grant:HouseGrantEconomicsV1,
});
impl HouseProductV1 {
    pub(super) fn validate(&self) -> Result<(), String> {
        self.grant.validate().map_err(error)?;
        if self.terms.seats().get() != self.protocol.seats()
            || self.grant.format() != self.protocol.format()
            || self.terms.form() != (PaidForm::Sealed {})
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
    pub(super) fn matches_subject(&self, s: &HouseSubjectV1) -> Result<(), String> {
        self.validate()?;
        s.validate()?;
        if self.terms.class() < s.minimum
            || self.revision
                != (RevisionPolicy::Pin {
                    filed: s.filed,
                    revision: s.revision,
                })
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
    pub const fn protocol(&self) -> HouseProtocol {
        self.protocol
    }
    pub fn grant(&self) -> &HouseGrantEconomicsV1 {
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
    subject: &HouseSubjectV1,
    product: &HouseProductV1,
    price: Amount,
    instrument: &InstrumentId,
    policy: &DefinitionRef,
    registry: &Cut,
    venue: &Venue,
) -> Result<(), String> {
    product.matches_subject(subject)?;
    if price.is_negative() || price.is_zero() {
        return Err(invalid());
    }
    product
        .grant
        .renewal()
        .matches_product(product.terms, product.revision, instrument, policy)
        .map_err(error)?;
    registry.validate()?;
    venue.cut.validate()?;
    let sum = u32::from(registry.0.permille) + u32::from(venue.cut.0.permille);
    if sum > 1000 {
        return Err(invalid());
    }
    Ok(())
}
data!(HouseListingV1,ListingWire,{
    format:u16,publication:CommitToken,publisher:ObjId,subject:HouseSubjectV1,product:HouseProductV1,
    price:Amount,instrument:InstrumentId,venue:Venue,registry:Cut,
    policy:DefinitionRef,observed_at_utc:i64,
});
impl HouseListingV1 {
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
}
impl Row for HouseListingV1 {
    const FAMILY: &'static str = "market_mind_house_listing";
    const MAX_BYTES: usize = MAX_LISTING_BYTES;
    fn key(&self) -> String {
        self.publication.to_hex()
    }
    fn validate(&self) -> Result<(), String> {
        if self.format != 1 || self.publisher.0 == 0 {
            return Err(invalid());
        }
        common(
            &self.subject,
            &self.product,
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
data!(HouseQuoteV1,QuoteWire,{
    format:u16,publication:CommitToken,listing:CommitToken,listing_digest:DefinitionDigest,
    buyer:ObjId,subject:HouseSubjectV1,product:HouseProductV1,product_digest:DefinitionDigest,
    price:Amount,instrument:InstrumentId,registry:Cut,venue:Venue,
    remainder:Recipient,payer:AccountId,payouts:Bounded<Payout,3>,
    policy:DefinitionRef,quoted_at_utc:i64,expires_at_utc:i64,
});
impl HouseQuoteV1 {
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
    pub(super) fn economic_data(
        &self,
        quote_digest: DefinitionDigest,
    ) -> Result<HousePurchase, String> {
        Ok(HousePurchase {
            quote: self.publication,
            quote_digest,
            product_digest: self.product_digest,
            buyer: self.buyer,
            organization: self.subject.issuer.organization(),
            scope: self.subject.issuer.scope(),
            charter: ObjId(self.subject.charter.get()),
            title_revision: self.subject.title_revision,
            payer: self.payer,
            instrument: self.instrument.clone(),
            price: self.price,
            registry: self.registry.0.clone(),
            venue: Some(self.venue.cut.0.clone()),
            remainder: self.remainder.clone(),
            grant_economics_digest: self.product.grant.digest().map_err(error)?,
            quoted_at_utc: self.quoted_at_utc,
            expires_at_utc: self.expires_at_utc,
        })
    }
    /// Pure E4 input, not an E4 result or account/current-rights capability.
    pub(super) fn economic_subject(&self) -> Result<HousePurchase, String> {
        self.validate()?;
        self.economic_data(self.raw_digest()?)
    }
}
impl Row for HouseQuoteV1 {
    const FAMILY: &'static str = "market_mind_house_quote";
    const MAX_BYTES: usize = MAX_QUOTE_BYTES;
    fn key(&self) -> String {
        self.publication.to_hex()
    }
    fn validate(&self) -> Result<(), String> {
        if self.format != 1
            || self.publication == self.listing
            || self.product_digest != self.product.digest()?
            || self.payouts.0.is_empty()
            || self.payouts.0.len() > 3
        {
            return Err(invalid());
        }
        common(
            &self.subject,
            &self.product,
            self.price,
            &self.instrument,
            &self.policy,
            &self.registry,
            &self.venue,
        )?;
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

impl HouseSubjectV1 {
    pub(super) fn historical(
        &self,
    ) -> mush_roll::commerce::house_purchase::writer::HouseListingIdentity {
        mush_roll::commerce::house_purchase::writer::HouseListingIdentity {
            issuer: self.issuer,
            charter: self.charter,
            line: self.line,
            filed: self.filed,
            sequence: self.sequence,
            owner_licence: self.owner_licence,
            owner_term: self.owner_term,
            owner_window: self.owner_window,
            revision: self.revision,
            schema: self.schema,
            definition: self.definition,
            rating: self.rating,
            compatibility: self.compatibility,
            declared: self.declared,
            minimum: self.minimum,
            title_revision: self.title_revision,
        }
    }
}

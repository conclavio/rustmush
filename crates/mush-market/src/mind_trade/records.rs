//! Strict market-owned observations. These rows are neither funded obligations
//! nor Roll grants. All ids are the actual action publication CommitToken; no
//! legacy market quote/listing allocator or second business identity is used.

use mush_core::{CommitToken, DefinitionDigest, ObjId};
use mush_economy::{AccountId, Amount, InstrumentId};
use mush_roll::commerce::rating::{RatingClass, RatingRef};
use mush_roll::commerce::sale_reader::OriginalSaleFacts;
use mush_roll::commerce::{
    CharterId, FiledRevisionId, LicenceId, LineId, RevisionIdentity, TermGeneration, TermWindow,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MAX_LISTING_BYTES: usize = 8 * 1024;
pub const MAX_QUOTE_BYTES: usize = 16 * 1024;
pub const MAX_QUOTE_SECONDS: i64 = 600;

/// Internal source-free identity, never a public disclosure whitelist. The
/// private Market typed rows contain no source or executable metadata.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OriginalSubject {
    pub(super) seller: ObjId,
    pub(super) charter: CharterId,
    pub(super) title_revision: u64,
    pub(super) line: LineId,
    pub(super) filed: FiledRevisionId,
    pub(super) sequence: u64,
    pub(super) owner_licence: LicenceId,
    pub(super) owner_term: TermGeneration,
    pub(super) owner_window: TermWindow,
    pub(super) revision: RevisionIdentity,
    pub(super) schema: DefinitionDigest,
    pub(super) definition: DefinitionDigest,
    pub(super) rating: RatingRef,
    pub(super) compatibility: DefinitionDigest,
    pub(super) declared: RatingClass,
    pub(super) minimum: RatingClass,
}
impl OriginalSubject {
    pub(super) fn capture(f: &OriginalSaleFacts) -> Self {
        Self {
            seller: f.seller(),
            charter: f.charter(),
            title_revision: f.title_revision(),
            line: f.line(),
            filed: f.filed(),
            sequence: f.sequence(),
            owner_licence: f.owner_licence(),
            owner_term: f.owner_term(),
            owner_window: f.owner_window(),
            revision: f.revision(),
            schema: f.schema(),
            definition: f.definition(),
            rating: f.rating(),
            compatibility: f.compatibility().digest(),
            declared: f.declared(),
            minimum: f.minimum(),
        }
    }
    fn validate(&self) -> Result<(), String> {
        if self.seller.0 == 0
            || self.title_revision == 0
            || self.sequence == 0
            || self.minimum < self.declared
        {
            return Err("invalid original quote subject".into());
        }
        Ok(())
    }
    pub const fn seller(&self) -> ObjId {
        self.seller
    }
    pub const fn charter(&self) -> CharterId {
        self.charter
    }
    pub const fn line(&self) -> LineId {
        self.line
    }
    pub const fn filed(&self) -> FiledRevisionId {
        self.filed
    }
    pub const fn revision(&self) -> RevisionIdentity {
        self.revision
    }
    pub const fn rating(&self) -> RatingRef {
        self.rating
    }
}

/// Closed original product. Explicit source form, one seat and a pinned filed
/// revision are explicit consent terms, not defaults omitted from its digest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OriginalProduct {
    pub(super) kind: ProductKind,
    pub(super) class: RatingClass,
    pub(super) seats: u16,
    pub(super) form: mush_roll::commerce::paid::PaidForm,
    pub(super) revision: mush_roll::commerce::records::RevisionPolicy,
    pub(super) transferable: bool,
    pub(super) term_seconds: i64,
    pub(super) grace_seconds: i64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum ProductKind {
    OriginalPersonal {},
    OriginalPersonalSourceForms {},
}
impl OriginalProduct {
    pub(super) fn protocol(&self) -> super::protocol::PurchaseProtocol {
        match self.kind {
            ProductKind::OriginalPersonal {} => super::protocol::PurchaseProtocol::SealedV1,
            ProductKind::OriginalPersonalSourceForms {} => {
                super::protocol::PurchaseProtocol::SourceFormsV2
            }
        }
    }
    pub(super) fn validate(&self, subject: &OriginalSubject) -> Result<(), String> {
        use mush_roll::commerce::paid::PaidForm;
        use mush_roll::commerce::records::RevisionPolicy;
        use mush_roll::commerce::{MAX_GRACE_SECONDS, MAX_TERM_SECONDS};
        if self.seats != 1
            || (self.protocol() == super::protocol::PurchaseProtocol::SealedV1
                && self.form != (PaidForm::Sealed {}))
            || self.class < subject.minimum
            || !(1..=MAX_TERM_SECONDS).contains(&self.term_seconds)
            || !(0..=MAX_GRACE_SECONDS).contains(&self.grace_seconds)
            || self.revision
                != (RevisionPolicy::Pin {
                    filed: subject.filed,
                    revision: subject.revision,
                })
        {
            return Err("invalid original personal product".into());
        }
        Ok(())
    }
    pub fn digest(&self) -> Result<DefinitionDigest, String> {
        digest(self.protocol().product_domain(), self, MAX_LISTING_BYTES)
    }
    pub const fn class(&self) -> RatingClass {
        self.class
    }
    pub const fn term_seconds(&self) -> i64 {
        self.term_seconds
    }
    pub const fn grace_seconds(&self) -> i64 {
        self.grace_seconds
    }
}

/// The exact frozen composition's recipient, rate and venue id. This is not a
/// claim that a venue admitted the actor. The real gate is consulted per action.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuoteVenue {
    pub(super) name: String,
    pub(super) recipient: mush_org::PrincipalId,
    pub(super) account: AccountId,
    pub(super) permille: u16,
}
impl QuoteVenue {
    pub fn new(
        name: crate::MarketId,
        recipient: mush_org::PrincipalId,
        account: AccountId,
        permille: u16,
    ) -> Result<Self, String> {
        let value = Self {
            name: name.as_str().into(),
            recipient,
            account,
            permille,
        };
        value.validate()?;
        Ok(value)
    }
    pub(super) fn validate(&self) -> Result<(), String> {
        if crate::MarketId::new(&self.name).is_none_or(|id| id.as_str() != self.name)
            || self.permille > 1000
        {
            return Err("invalid quote venue".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MindListing {
    pub(super) format: u16,
    pub(super) publication: CommitToken,
    pub(super) subject: OriginalSubject,
    pub(super) product: OriginalProduct,
    pub(super) price: Amount,
    pub(super) instrument: InstrumentId,
    pub(super) venue: QuoteVenue,
    pub(super) registry_recipient: mush_org::SystemPrincipalId,
    pub(super) registry_account: AccountId,
    pub(super) registry_permille: u16,
    pub(super) policy: DefinitionDigest,
    pub(super) observed_at_utc: i64,
}
impl MindListing {
    pub fn publication(&self) -> CommitToken {
        self.publication
    }
    pub fn subject(&self) -> &OriginalSubject {
        &self.subject
    }
    pub fn product(&self) -> &OriginalProduct {
        &self.product
    }
    pub fn digest(&self) -> Result<DefinitionDigest, String> {
        digest(
            self.product.protocol().listing_domain(),
            self,
            MAX_LISTING_BYTES,
        )
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        decode_bytes(bytes, MAX_LISTING_BYTES)
    }
}

/// Exact integer amounts; no floating/percentage settlement or implicit
/// remainder survives into the quote. Account ids here are E4 wallet ids, not
/// the private authentication AccountIdentity values.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuotePayout {
    pub(super) account: AccountId,
    pub(super) amount: Amount,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MindQuote {
    pub(super) format: u16,
    pub(super) publication: CommitToken,
    pub(super) listing: CommitToken,
    pub(super) listing_digest: DefinitionDigest,
    pub(super) buyer: ObjId,
    pub(super) subject: OriginalSubject,
    pub(super) product: OriginalProduct,
    pub(super) product_digest: DefinitionDigest,
    pub(super) price: Amount,
    pub(super) instrument: InstrumentId,
    pub(super) venue: QuoteVenue,
    pub(super) registry_recipient: mush_org::SystemPrincipalId,
    pub(super) registry_account: AccountId,
    pub(super) registry_permille: u16,
    pub(super) payer: AccountId,
    pub(super) seller_account: AccountId,
    pub(super) payouts: Vec<QuotePayout>,
    pub(super) policy: DefinitionDigest,
    pub(super) quoted_at_utc: i64,
    pub(super) expires_at_utc: i64,
}
impl MindQuote {
    /// Exact retained issuance rates for a later property sale. This is a data
    /// projection only; the resale owner must prove this quote digest against
    /// actual issuance history and recheck current recipient account bindings.
    pub(crate) fn resale_rates(
        &self,
    ) -> Result<
        (
            InstrumentId,
            mush_economy::managed_entitlement::GrossRecipientCut,
            Option<mush_economy::managed_entitlement::GrossRecipientCut>,
        ),
        String,
    > {
        <Self as Row>::validate(self)?;
        use mush_economy::managed_entitlement::{GrossRecipientCut, Recipient};
        Ok((
            self.instrument.clone(),
            GrossRecipientCut {
                permille: self.registry_permille,
                recipient: (self.registry_permille != 0).then(|| Recipient {
                    account: self.registry_account,
                    owner: mush_org::PrincipalId::System(self.registry_recipient.clone()),
                }),
            },
            Some(GrossRecipientCut {
                permille: self.venue.permille,
                recipient: (self.venue.permille != 0).then(|| Recipient {
                    account: self.venue.account,
                    owner: self.venue.recipient.clone(),
                }),
            }),
        ))
    }
    pub fn publication(&self) -> CommitToken {
        self.publication
    }
    pub fn buyer(&self) -> ObjId {
        self.buyer
    }
    pub fn subject(&self) -> &OriginalSubject {
        &self.subject
    }
    pub fn product(&self) -> &OriginalProduct {
        &self.product
    }
    pub fn digest(&self) -> Result<DefinitionDigest, String> {
        digest(
            self.product.protocol().quote_domain(),
            self,
            MAX_QUOTE_BYTES,
        )
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        decode_bytes(bytes, MAX_QUOTE_BYTES)
    }
}

pub(super) trait Row:
    Clone + Serialize + for<'de> Deserialize<'de> + Send + Sync + 'static
{
    const FAMILY: &'static str;
    const MAX_BYTES: usize;
    fn validate(&self) -> Result<(), String>;
}
impl Row for MindListing {
    const FAMILY: &'static str = "market_mind_original_listing";
    const MAX_BYTES: usize = MAX_LISTING_BYTES;
    fn validate(&self) -> Result<(), String> {
        self.subject.validate()?;
        self.product.validate(&self.subject)?;
        self.venue.validate()?;
        if u32::from(self.format) != self.product.protocol().version()
            || self.price.is_negative()
            || self.price.is_zero()
            || u32::from(self.registry_permille) + u32::from(self.venue.permille) > 1000
        {
            return Err("invalid original listing".into());
        }
        Ok(())
    }
}
impl Row for MindQuote {
    const FAMILY: &'static str = "market_mind_original_quote";
    const MAX_BYTES: usize = MAX_QUOTE_BYTES;
    fn validate(&self) -> Result<(), String> {
        self.subject.validate()?;
        self.product.validate(&self.subject)?;
        self.venue.validate()?;
        let seconds = self
            .expires_at_utc
            .checked_sub(self.quoted_at_utc)
            .ok_or("quote expiry overflow")?;
        if u32::from(self.format) != self.product.protocol().version()
            || self.buyer.0 == 0
            || self.buyer == self.subject.seller
            || !(1..=MAX_QUOTE_SECONDS).contains(&seconds)
            || self.price.is_negative()
            || self.price.is_zero()
            || self.product.digest()? != self.product_digest
            || self.payer == self.seller_account
        {
            return Err("invalid original quote".into());
        }
        let exact = mush_roll::commerce::proceeds::calculate(
            self.price,
            mush_roll::commerce::proceeds::GrossCut {
                to: self.registry_account,
                permille: self.registry_permille,
            },
            Some(mush_roll::commerce::proceeds::GrossCut {
                to: self.venue.account,
                permille: self.venue.permille,
            }),
            &[],
            self.seller_account,
        )
        .map_err(|e| e.to_string())?;
        if payouts(exact)? != self.payouts || self.payouts.iter().any(|p| p.account == self.payer) {
            return Err("quote payout does not match its original gross terms".into());
        }
        Ok(())
    }
}
pub(super) fn payouts(
    exact: mush_roll::commerce::proceeds::ExactProceeds,
) -> Result<Vec<QuotePayout>, String> {
    exact
        .into_shares()
        .into_iter()
        .map(|share| match share.share {
            mush_economy::Share::Exact(amount) => Ok(QuotePayout {
                account: share.to,
                amount,
            }),
            _ => Err("original proceeds must be exact".into()),
        })
        .collect()
}
pub(super) fn decode_value<R: Row>(value: &Value) -> Result<R, String> {
    let row: R = serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
    row.validate()?;
    // Strictness also covers shared nested codecs which normalize names or
    // otherwise ignore unknown fields. Unknown input never silently disappears.
    if serde_json::to_value(&row).map_err(|e| e.to_string())? != *value {
        return Err("noncanonical or unknown original quote field".into());
    }
    Ok(row)
}
pub(super) fn decode_bytes<R: Row>(bytes: &[u8], limit: usize) -> Result<R, String> {
    if bytes.is_empty() || bytes.len() > limit {
        return Err("original quote document exceeds bound".into());
    }
    // Decode the typed record before constructing Value: Value would collapse
    // duplicate fields and conceal an ambiguous persisted consent document.
    let row: R = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    row.validate()?;
    let input: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if serde_json::to_value(&row).map_err(|e| e.to_string())? != input {
        return Err("noncanonical or unknown original quote field".into());
    }
    Ok(row)
}
pub(super) fn digest<T: Serialize>(
    domain: &str,
    row: &T,
    limit: usize,
) -> Result<DefinitionDigest, String> {
    let value = serde_json::to_value(row).map_err(|e| e.to_string())?;
    DefinitionDigest::for_canonical_value_with_limit(domain, &value, limit)
        .map_err(|e| e.to_string())
}

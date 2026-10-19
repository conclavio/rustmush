//! Closed consent protocol identity, never a source-access capability.
use mush_roll::commerce::paid::PaidForm;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PurchaseProtocol {
    SealedV1,
    SourceFormsV2,
}
impl PurchaseProtocol {
    pub(super) const fn version(self) -> u32 {
        self.format() as u32
    }
    pub(super) const fn format(self) -> u16 {
        match self {
            Self::SealedV1 => 1,
            Self::SourceFormsV2 => 2,
        }
    }
    pub(super) fn payload(self, transition: &str) -> Value {
        json!({"format":self.version(),"protocol":match self { Self::SealedV1 => "original_mind_purchase_v1", Self::SourceFormsV2 => "original_mind_purchase_v2" },"transition":transition})
    }
    pub(super) const fn product_domain(self) -> &'static str {
        match self {
            Self::SealedV1 => "market.original_product.v1",
            Self::SourceFormsV2 => "market.original_product.v2",
        }
    }
    pub(super) const fn listing_domain(self) -> &'static str {
        match self {
            Self::SealedV1 => "market.original_listing.v1",
            Self::SourceFormsV2 => "market.original_listing.v2",
        }
    }
    pub(super) const fn quote_domain(self) -> &'static str {
        match self {
            Self::SealedV1 => "market.original_quote.v1",
            Self::SourceFormsV2 => "market.original_quote.v2",
        }
    }
}

/// Required v2 listing consent. JSON strings map exactly to signed choices;
/// this data never authorizes a source read or an owner mutation.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum SourceForm {
    Sealed,
    Open,
    OpenResale,
}
impl SourceForm {
    pub(super) const fn paid(self) -> PaidForm {
        match self {
            Self::Sealed => PaidForm::Sealed {},
            Self::Open => PaidForm::Open {},
            Self::OpenResale => PaidForm::OpenResale {},
        }
    }
}

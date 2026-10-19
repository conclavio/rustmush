//! Accepted issuance economics, with no fresh issuer/parent rights gate.
//! Positive parent account/title bindings are resolved by Market at Offer;
//! these immutable rates never come from today's catalogue.
use super::{history, records::digest};
use crate::commerce::{
    entitlement::history::{self as acquired, Birth, read},
    families::CommerceFamilies,
    paid::{PaidTermRecord, PersonalPaidLicence},
    rating::RatingFamilies,
    read::CommerceRead,
    records::{PublicationError, Record},
};
use mush_core::{CommitToken, DefinitionDigest, ObjId};
use mush_economy::{
    EconomyStore, InstrumentId,
    managed_entitlement::{GrossRecipientCut, RoyaltyRecipientCut},
    managed_purchase::OriginalPurchase,
};
use serde::Serialize;
fn bad(_: impl std::fmt::Display) -> PublicationError {
    PublicationError::Mismatch
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ResaleParentRate {
    charter: ObjId,
    permille: u16,
}
impl ResaleParentRate {
    pub const fn charter(&self) -> ObjId {
        self.charter
    }
    pub const fn permille(&self) -> u16 {
        self.permille
    }
}
/// Legacy rates must be read from the exact retained Market quote. Merged native
/// payout amounts cannot uniquely identify those rates and are never inverted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResaleRates {
    Legacy {
        quote: CommitToken,
        quote_digest: DefinitionDigest,
    },
    Accepted {
        registry: GrossRecipientCut,
        venue: Option<GrossRecipientCut>,
        parents: Vec<ResaleParentRate>,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ResaleEconomics {
    instrument: InstrumentId,
    economics_digest: DefinitionDigest,
    royalty_chain_digest: DefinitionDigest,
    rates: ResaleRates,
}
impl ResaleEconomics {
    pub fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }
    pub const fn economics_digest(&self) -> DefinitionDigest {
        self.economics_digest
    }
    pub const fn royalty_chain_digest(&self) -> DefinitionDigest {
        self.royalty_chain_digest
    }
    pub fn rates(&self) -> &ResaleRates {
        &self.rates
    }
}
fn legacy_digest(v: &OriginalPurchase) -> Result<DefinitionDigest, PublicationError> {
    let payouts: Vec<_> = v
        .payouts
        .iter()
        .map(|p| serde_json::json!({"account":p.account,"owner":p.owner,"amount":p.amount}))
        .collect();
    digest(
        "roll.paid_resale.legacy_economics.v1",
        &serde_json::json!({
            "quote":v.quote,"quote_digest":v.quote_digest,"product_digest":v.product_digest,
            "buyer":v.buyer,"seller":v.seller,"charter":v.charter,"title_revision":v.title_revision,
            "payer":v.payer,"instrument":v.instrument,"price":v.price,"payouts":payouts,
            "quoted_at_utc":v.quoted_at_utc,"expires_at_utc":v.expires_at_utc
        }),
    )
}
fn rates(
    registry: &GrossRecipientCut,
    venue: Option<&GrossRecipientCut>,
    parents: &[RoyaltyRecipientCut],
) -> Result<ResaleRates, PublicationError> {
    if parents.len() > 3 {
        return Err(PublicationError::Capacity);
    }
    Ok(ResaleRates::Accepted {
        registry: registry.clone(),
        venue: venue.cloned(),
        parents: parents
            .iter()
            .map(|r| ResaleParentRate {
                charter: r.charter,
                permille: r.permille,
            })
            .collect(),
    })
}
/// Caller supplies the actual current or retained pointer; this routine proves
/// the requested issued term independently of today's holder and eligibility.
pub(crate) fn accepted(
    ctx: &impl CommerceRead,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
    licence: &PersonalPaidLicence,
    term: &PaidTermRecord,
) -> Result<ResaleEconomics, PublicationError> {
    let proved = acquired::granted(ctx, f, ratings, licence, term)?;
    let e = ctx
        .world()
        .module_state::<EconomyStore>()
        .ok_or(PublicationError::Unavailable)?;
    let audit = e.purchase_receipt_audit();
    audit.require_complete().map_err(bad)?;
    let actual = audit.get(licence.operation()).map_err(bad)?;
    match proved.birth() {
        Birth::Legacy(_) => {
            let p = actual
                .original()
                .map_err(bad)?
                .ok_or(PublicationError::Missing)?;
            let birth = history::birth(ctx, f, ratings, licence)?;
            Ok(ResaleEconomics {
                instrument: p.instrument.clone(),
                economics_digest: legacy_digest(&p)?,
                royalty_chain_digest: digest(
                    "roll.paid_resale.legacy_royalty_basis.v1",
                    &serde_json::json!({"format":1,"birth":birth.reference,"parents":[]}),
                )?,
                rates: ResaleRates::Legacy {
                    quote: p.quote,
                    quote_digest: p.quote_digest,
                },
            })
        }
        Birth::Entitlement(_) => {
            let economics = proved.economics().ok_or(PublicationError::Missing)?;
            let chain = read(ctx, &f.filed_royalties, proved.filed().key())?;
            if chain.reference()? != economics.grant().royalty_chain() {
                return Err(PublicationError::Mismatch);
            }
            let (instrument, selected) = if term.generation().get() == 1 {
                let p = actual
                    .entitlement()
                    .map_err(bad)?
                    .ok_or(PublicationError::Missing)?;
                (
                    p.instrument.clone(),
                    rates(&p.registry, p.venue.as_ref(), &p.parents)?,
                )
            } else {
                let view = e
                    .renewal_audit()
                    .map_err(bad)?
                    .get(term.operation())
                    .map_err(bad)?;
                let t = &view.offer().terms;
                (
                    t.instrument.clone(),
                    rates(&t.registry, t.venue.as_ref(), &t.parents)?,
                )
            };
            let ResaleRates::Accepted { parents, .. } = &selected else {
                return Err(PublicationError::Mismatch);
            };
            if parents.len() != chain.edges().len()
                || parents.iter().zip(chain.edges()).any(|(a, b)| {
                    a.charter != b.charter().object() || a.permille != b.royalty_permille()
                })
            {
                return Err(PublicationError::Mismatch);
            }
            Ok(ResaleEconomics {
                instrument,
                economics_digest: economics.digest()?,
                royalty_chain_digest: chain.digest()?,
                rates: selected,
            })
        }
        Birth::House(_) => {
            let economics = read(ctx, &f.house_economics, term.key())?;
            let birth = history::birth(ctx, f, ratings, licence)?;
            let (instrument, selected) = if term.generation().get() == 1 {
                let p = actual
                    .house()
                    .map_err(bad)?
                    .ok_or(PublicationError::Missing)?;
                (
                    p.instrument.clone(),
                    rates(&p.registry, p.venue.as_ref(), &[])?,
                )
            } else {
                let view = e
                    .house_renewal_audit()
                    .map_err(bad)?
                    .get(term.operation())
                    .map_err(bad)?;
                let t = &view.offer().terms;
                (
                    t.instrument.clone(),
                    rates(&t.registry, t.venue.as_ref(), &[])?,
                )
            };
            Ok(ResaleEconomics {
                instrument,
                economics_digest: economics.digest()?,
                royalty_chain_digest: digest(
                    "roll.paid_resale.house_royalty_basis.v1",
                    &serde_json::json!({"format":1,"birth":birth.reference,"parents":[]}),
                )?,
                rates: selected,
            })
        }
    }
}

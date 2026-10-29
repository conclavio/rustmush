//! Actual operator consent, retained before the buyer can accept fixed work.
//! This immutable row is not a hold and does not make a physical lot available.
use super::{IntegrationError as Error, PublishedCoreRecipe, bad};
use crate::core_production::records::DieCertificate;
use crate::facility::FacilityRecord;
use mush_core::{ClockRef, ClockStamp, CommitToken, DefinitionDigest, DefinitionRef, ObjId};
use mush_economy::core_integration::{CoreClass, CorePaymentPolicy, IntegrationSite};
use mush_inventory::Lot;
use serde::{Deserialize, Serialize};

pub(crate) const OFFER_SECONDS: i64 = 300;
pub(crate) const OFFER_BYTES: usize = 16_384;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum OfferedInput {
    /// The operator has actual current withdrawal authority for this exact lot.
    SupplierStock {
        certificate: Box<DieCertificate>,
        lot: Box<Lot>,
    },
    /// The operator consents to service for the named recipient/class only.
    /// Buyer's acceptance later supplies its own exact certified input; this
    /// offer does not inspect or authorize a guessed private customer vault.
    RequesterSupplied {},
    /// **The ungoverning site's input** (doc 17h cut M8b-2, H2): the operator of the facility
    /// holding `core_ungovern` consents to integrate, for the named recipient and class, a
    /// heirloom or grey die the buyer's acceptance supplies with no certificate.
    Uncertified {},
}
impl OfferedInput {
    /// `false` for the ungoverning site's input: the site a die of this input integrates at.
    pub(crate) const fn certified(&self) -> bool {
        !matches!(self, Self::Uncertified {})
    }
}

/// **Whether `facility` is `site`'s** (doc 17h cut M8b-2, H2): the site's object, owned by its
/// integrator's organization, holding its capability; the ungoverning site's facility holds
/// neither `core_integration` nor `core_refit`.
pub(crate) fn holds_site(facility: &FacilityRecord, site: &IntegrationSite<'_>) -> bool {
    let holds = |word: &str| facility.capabilities().iter().any(|c| c.as_str() == word);
    facility.object() == site.factory
        && facility.owner() == &mush_org::PrincipalId::Organization(site.integrator.organization)
        && holds(site.capability)
        && (site.capability != "core_ungovern"
            || (!holds("core_integration") && !holds("core_refit")))
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FactoryOffer {
    pub(crate) format: u16,
    pub(crate) sequence: u64,
    pub(crate) operation: CommitToken,
    pub(crate) action: DefinitionRef,
    pub(crate) operator: ObjId,
    pub(crate) requester: ObjId,
    pub(crate) policy: CorePaymentPolicy,
    pub(crate) facility: FacilityRecord,
    pub(crate) recipe: PublishedCoreRecipe,
    pub(crate) input: OfferedInput,
    pub(crate) observed_ic: ClockStamp,
    pub(crate) observed_utc: i64,
    pub(crate) expires_utc: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OfferWire {
    format: u16,
    sequence: u64,
    operation: CommitToken,
    action: DefinitionRef,
    operator: ObjId,
    requester: ObjId,
    policy: CorePaymentPolicy,
    facility: FacilityRecord,
    recipe: PublishedCoreRecipe,
    input: OfferedInput,
    observed_ic: ClockStamp,
    observed_utc: i64,
    expires_utc: i64,
}
impl<'de> Deserialize<'de> for FactoryOffer {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let w = OfferWire::deserialize(d)?;
        let offer = Self {
            format: w.format,
            sequence: w.sequence,
            operation: w.operation,
            action: w.action,
            operator: w.operator,
            requester: w.requester,
            policy: w.policy,
            facility: w.facility,
            recipe: w.recipe,
            input: w.input,
            observed_ic: w.observed_ic,
            observed_utc: w.observed_utc,
            expires_utc: w.expires_utc,
        };
        offer.validate().map_err(serde::de::Error::custom)?;
        Ok(offer)
    }
}
impl FactoryOffer {
    pub(crate) fn class(&self) -> CoreClass {
        match self.recipe.class() {
            crate::core_production::DieClass::I => CoreClass::I,
            crate::core_production::DieClass::II => CoreClass::II,
            crate::core_production::DieClass::III => CoreClass::III,
        }
    }
    pub(crate) fn validate(&self) -> Result<(), Error> {
        self.policy.validate().map_err(bad)?;
        self.recipe.validate()?;
        if self.format != 1
            || !(1..=8192).contains(&self.sequence)
            || self.action
                != self
                    .policy
                    .action_ref("offer_core_integration")
                    .map_err(bad)?
            || !holds_site(
                &self.facility,
                &self.policy.site(self.input.certified()).map_err(bad)?,
            )
            || self.recipe.definition() != &self.policy.recipe
            || self.observed_utc.checked_add(OFFER_SECONDS) != Some(self.expires_utc)
            || !matches!(self.observed_ic.clock, ClockRef::WorldIc { generation } if generation > 0)
        {
            return Err(Error::Invalid);
        }
        if let OfferedInput::SupplierStock { certificate, lot } = &self.input {
            certificate.matches_lot(lot).map_err(bad)?;
            if certificate.class != self.recipe.class()
                || certificate.policy.importer != self.policy.supplier.organization
                || !self.policy.supplier_stock.contains(&lot.endpoint().0)
            {
                return Err(Error::Integrity);
            }
        }
        let value = serde_json::to_value(self).map_err(bad)?;
        mush_core::mechanics::canonical_json_bytes_with_limit(&value, OFFER_BYTES)
            .map_err(|_| Error::Capacity)?;
        Ok(())
    }

    pub(crate) fn digest(&self) -> Result<DefinitionDigest, Error> {
        self.validate()?;
        let value = serde_json::to_value(self).map_err(bad)?;
        mush_core::mechanics::canonical_json_bytes_with_limit(&value, OFFER_BYTES)
            .map_err(|_| Error::Capacity)?;
        DefinitionDigest::for_canonical_value("industry.core_integration_offer.v1", &value)
            .map_err(bad)
    }

    pub(crate) fn require_current_time(
        &self,
        now_ic: &ClockStamp,
        now_utc: i64,
    ) -> Result<(), Error> {
        self.validate()?;
        if now_ic.clock != self.observed_ic.clock
            || now_ic.at < self.observed_ic.at
            || now_utc < self.observed_utc
            || now_utc >= self.expires_utc
        {
            return Err(Error::Expired);
        }
        Ok(())
    }
}

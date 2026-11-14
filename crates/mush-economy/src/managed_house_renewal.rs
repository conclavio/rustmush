//! The House renewal family: an organisation's mind licence, renewed.
//!
//! The owner is `crate::renewal_twin`, written once for both issuers. This module is its
//! House instantiation: the House issuer's identity strings and the rules that are its
//! alone, the records its offers are made of (`subject.rs`), and the family's face under the
//! paths the rest of Economy, the Roll and the market name.
//! Paired Roll/Market capacity and final audits remain separate prerequisites;
//! default composition grants no renewal action.

use crate::EconomySnapshot;
use crate::contract::managed::ManagedConsent;
use crate::engine::ValidateEnv;
use crate::fold::FoldState;
use crate::managed_paid_resale::PaidBirthRouteV1;
use crate::module::EconomyStore;
use crate::plan::PlanResult;
use crate::renewal_twin::{self, House, Issuer};
use mush_core::ObjId;
use mush_core::participant::ParticipantError;
use mush_org::OrgScopeId;
use std::collections::BTreeSet;

#[cfg(test)]
mod bridge_registration_tests;
mod subject;
pub use subject::{RenewalBasisV1, RenewalOffer, RenewalOfferTermsV1};

impl Issuer for House {
    const DRAFT_ACTION: &'static str = "draft_house_mind_renewal";
    const CLOSE_ACTION: &'static str = "close_house_mind_renewal";
    const FUND_ACTION: &'static str = "fund_house_mind_renewal";
    const GRANT_ACTION: &'static str = "settle_house_mind_renewal";
    const REFUND_ACTION: &'static str = "refund_house_mind_renewal";
    const SERVICE_CLOSE_ACTION: &'static str = "close_house_mind_renewal_service";
    const NAMESPACE: &'static str = "market.mind.house_renewal.v1";
    const OFFER_PREFIX: &'static str = "house-renewal-offer:";
    const SERVICE_PREFIX: &'static str = "house-renewal-service-close:";
    const PROTOCOL: &'static str = "house_mind_renewal_v1";
    const SERVICE_PROTOCOL: &'static str = "house_mind_renewal_service_v1";
    const OFFER_DOMAIN: &'static str = "economy.house_mind_renewal_offer.v1";
    const CONTRACT_NAME: &'static str = "House mind renewal offer";
    const FUNDING_DESCRIPTION: &'static str = "house mind renewal funding";
    const BIRTH_ROUTE: PaidBirthRouteV1 = PaidBirthRouteV1::House;

    type Basis = RenewalBasisV1;
    type Terms = RenewalOfferTermsV1;
    type Offer = RenewalOffer;

    fn book(state: &EconomyStore) -> &renewal_twin::history::OfferBook<Self> {
        &state.house_renewal_offers
    }
    fn bind(
        state: &mut EconomyStore,
        _: renewal_twin::BindKey,
    ) -> (
        &mut EconomySnapshot,
        &mut renewal_twin::history::OfferBook<Self>,
        &mut u64,
    ) {
        (
            &mut state.snapshot,
            &mut state.house_renewal_offers,
            &mut state.next_contract,
        )
    }
    fn receipts(fold: &FoldState) -> &renewal_twin::receipt::RenewalReceiptAudit<Self> {
        &fold.house_renewal_receipts
    }
    fn receipts_mut(fold: &mut FoldState) -> &mut renewal_twin::receipt::RenewalReceiptAudit<Self> {
        &mut fold.house_renewal_receipts
    }
    fn inventory_checked(state: &EconomyStore) -> bool {
        state.house_renewal_inventory_checked
    }
    fn marker(consent: &ManagedConsent) -> Option<&renewal_twin::RenewalMarkerV1<Self>> {
        consent.house_renewal_offer_marker()
    }
    fn consent(
        marker: renewal_twin::RenewalMarkerV1<Self>,
    ) -> Result<ManagedConsent, ParticipantError> {
        ManagedConsent::house_renewal_offer(marker)
    }
    fn validate(
        proof: &renewal_twin::ClosedPlan<Self>,
        snapshot: &EconomySnapshot,
        env: &ValidateEnv<'_>,
    ) -> PlanResult {
        crate::engine::validate_house_renewal(proof, snapshot, env)
    }

    /// A House renewal is filed under the issuing organisation's scope.
    fn contract_scope(terms: &RenewalOfferTermsV1) -> Option<OrgScopeId> {
        Some(terms.scope)
    }
    /// The charter alone: a House offer has no royalty parent.
    fn titles(terms: &RenewalOfferTermsV1) -> BTreeSet<ObjId> {
        BTreeSet::from([terms.charter])
    }
    /// A retry keeps the organisation and the scope of the offer it follows.
    fn same_line(row: &RenewalOfferTermsV1, before: &RenewalOfferTermsV1) -> bool {
        row.organization == before.organization && row.scope == before.scope
    }
}

renewal_twin::entry_points!(crate::renewal_twin::House, ClosedHouseRenewalPlan);

#[cfg(test)]
mod ct0_snapshot;
#[cfg(test)]
mod tests;

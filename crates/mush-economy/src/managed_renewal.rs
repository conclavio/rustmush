//! The personal renewal family: a player's own mind licence, renewed.
//!
//! The owner is `crate::renewal_twin`, written once for both issuers. This module is its
//! personal instantiation: the personal issuer's identity strings and the rules that are its
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
use crate::renewal_twin::{self, Issuer, Personal};
use mush_core::ObjId;
use mush_core::participant::ParticipantError;
use mush_org::OrgScopeId;
use std::collections::BTreeSet;

#[cfg(test)]
mod bridge_registration_tests;
mod subject;
pub use subject::{RenewalBasisV1, RenewalLineageV1, RenewalOffer, RenewalOfferTermsV1};

impl Issuer for Personal {
    const DRAFT_ACTION: &'static str = "draft_mind_renewal";
    const CLOSE_ACTION: &'static str = "close_mind_renewal";
    const FUND_ACTION: &'static str = "fund_mind_renewal";
    const GRANT_ACTION: &'static str = "settle_mind_renewal";
    const REFUND_ACTION: &'static str = "refund_mind_renewal";
    const SERVICE_CLOSE_ACTION: &'static str = "close_mind_renewal_service";
    const NAMESPACE: &'static str = "market.mind.renewal.v1";
    const OFFER_PREFIX: &'static str = "renewal-offer:";
    const SERVICE_PREFIX: &'static str = "renewal-service-close:";
    const PROTOCOL: &'static str = "personal_mind_renewal_v1";
    const SERVICE_PROTOCOL: &'static str = "personal_mind_renewal_service_v1";
    const OFFER_DOMAIN: &'static str = "economy.mind_renewal_offer.v1";
    const CONTRACT_NAME: &'static str = "Mind renewal offer";
    const FUNDING_DESCRIPTION: &'static str = "mind renewal funding";
    const BIRTH_ROUTE: PaidBirthRouteV1 = PaidBirthRouteV1::Entitlement;

    type Basis = RenewalBasisV1;
    type Terms = RenewalOfferTermsV1;
    type Offer = RenewalOffer;

    fn book(state: &EconomyStore) -> &renewal_twin::history::OfferBook<Self> {
        &state.renewal_offers
    }
    fn bind(
        state: &mut EconomyStore,
    ) -> (
        &mut EconomySnapshot,
        &mut renewal_twin::history::OfferBook<Self>,
        &mut u64,
    ) {
        (
            &mut state.snapshot,
            &mut state.renewal_offers,
            &mut state.next_contract,
        )
    }
    fn receipts(fold: &FoldState) -> &renewal_twin::receipt::RenewalReceiptAudit<Self> {
        &fold.renewal_receipts
    }
    fn receipts_mut(fold: &mut FoldState) -> &mut renewal_twin::receipt::RenewalReceiptAudit<Self> {
        &mut fold.renewal_receipts
    }
    fn inventory_checked(state: &EconomyStore) -> bool {
        state.renewal_inventory_checked
    }
    fn marker(consent: &ManagedConsent) -> Option<&renewal_twin::RenewalMarkerV1<Self>> {
        consent.renewal_offer_marker()
    }
    fn consent(
        marker: renewal_twin::RenewalMarkerV1<Self>,
    ) -> Result<ManagedConsent, ParticipantError> {
        ManagedConsent::renewal_offer(marker)
    }
    fn validate(
        proof: &renewal_twin::ClosedPlan<Self>,
        snapshot: &EconomySnapshot,
        env: &ValidateEnv<'_>,
    ) -> PlanResult {
        crate::engine::validate_renewal(proof, snapshot, env)
    }

    /// A personal renewal is filed under no scope.
    fn contract_scope(_: &RenewalOfferTermsV1) -> Option<OrgScopeId> {
        None
    }
    /// The charter, and the charter of every royalty parent that is paid.
    fn titles(terms: &RenewalOfferTermsV1) -> BTreeSet<ObjId> {
        let mut titles = BTreeSet::from([terms.charter]);
        titles.extend(
            terms
                .parents
                .iter()
                .filter(|p| p.title.is_some())
                .map(|p| p.charter),
        );
        titles
    }
    /// A retry keeps the lineage of the offer it follows.
    fn same_line(row: &RenewalOfferTermsV1, before: &RenewalOfferTermsV1) -> bool {
        row.lineage == before.lineage
    }
}

renewal_twin::entry_points!(crate::renewal_twin::Personal, ClosedRenewalPlan);

#[cfg(test)]
mod tests;

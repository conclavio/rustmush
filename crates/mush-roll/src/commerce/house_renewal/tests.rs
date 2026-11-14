//! Structural history tests. These do not fabricate an accepted E4 grant;
//! actual owner/output/audit integration must also pass at the paired boundary.
//!
//! Doc 46 CT.5: the scenarios are the twin's (`renewal_twin::tests`), run here for the House
//! issuer under their old names; this module holds the House rig and the CT.0 walk.
use super::super::renewal_twin::tests::{self as twin, Rig};
use super::super::{
    Holder, LicenceId, RevisionIdentity, TermGeneration,
    house_catalogue::HouseIssuer,
    house_purchase::{HouseGrantEconomicsV1, HouseRenewalTemplateV1, HouseTermEconomicsV1},
    paid::{PaidForm, PaidTermRecord, PaidTerms, PersonalPaidLicence},
    rating::RatingClass,
    records::{PublicationError, Record, RevisionPolicy},
    renewal_twin::House,
};
use super::records::*;
use super::*;
use mush_core::{CommitToken, DefinitionDigest, DefinitionRef, ObjId};
use mush_economy::{Amount, InstrumentId};
#[allow(unused_imports)]
use std::collections::BTreeMap;
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
use mush_testkit::fixtures::digest;
#[allow(unused_imports)]
use mush_testkit::fixtures::stamp;
use mush_testkit::fixtures::token;

/// The House of the structural birth.
fn issuer() -> TestResult<HouseIssuer> {
    Ok(HouseIssuer::new(
        mush_org::OrgId(0),
        mush_org::OrgScopeId(0),
    )?)
}

impl Rig for House {
    fn birth_terms() -> TestResult<PaidTerms> {
        Ok(PaidTerms::original_personal_form(
            RatingClass::I,
            PaidForm::Sealed {},
            false,
        )?)
    }
    fn grant(terms: PaidTerms, policy: RevisionPolicy) -> TestResult<HouseGrantEconomicsV1> {
        Ok(HouseGrantEconomicsV1::new(HouseRenewalTemplateV1::new(
            InstrumentId::new("cr").ok_or("instrument")?,
            Amount::from_minor(500),
            100,
            20,
            terms,
            policy,
            twin::definition("entitlement_policy", "house_personal")?,
        )?)?)
    }
    fn widen(
        old: &HouseGrantEconomicsV1,
        instrument: InstrumentId,
    ) -> TestResult<HouseGrantEconomicsV1> {
        let t = old.renewal();
        let template = HouseRenewalTemplateV1::new(
            instrument,
            t.price(),
            t.term_seconds(),
            t.grace_seconds(),
            t.terms(),
            t.revision(),
            t.policy().clone(),
        )?;
        Ok(HouseGrantEconomicsV1::new(template)?)
    }
    fn capture(
        licence: &PersonalPaidLicence,
        term: &PaidTermRecord,
        economics: &HouseTermEconomicsV1,
    ) -> Result<RenewalBindingV1, PublicationError> {
        RenewalBindingV1::capture(
            licence,
            term,
            economics,
            issuer().map_err(|_| PublicationError::Invalid)?,
        )
    }
    fn birth_line() -> TestResult<HouseIssuer> {
        issuer()
    }
    fn other_line() -> TestResult<Option<HouseIssuer>> {
        Ok(Some(HouseIssuer::new(
            mush_org::OrgId(1),
            mush_org::OrgScopeId(0),
        )?))
    }
    fn economics_row(
        licence: LicenceId,
        generation: TermGeneration,
        operation: CommitToken,
        product: DefinitionDigest,
        grant: HouseGrantEconomicsV1,
    ) -> Result<HouseTermEconomicsV1, PublicationError> {
        HouseTermEconomicsV1::capture(licence, generation, operation, product, grant)
    }
    fn product(economics: &HouseTermEconomicsV1) -> DefinitionDigest {
        economics.product_digest()
    }
    fn grant_of(economics: &HouseTermEconomicsV1) -> &HouseGrantEconomicsV1 {
        economics.grant()
    }
    fn with_holder(row: &RenewalOperationV1, holder: Holder) -> RenewalOperationV1 {
        let mut row = row.clone();
        row.binding.holder = holder;
        row
    }
    fn with_attempt(row: &RenewalOperationV1, attempt: RenewalAttemptV1) -> RenewalOperationV1 {
        let mut row = row.clone();
        row.attempt = attempt;
        row
    }
    fn with_offer_digest(row: &RenewalOperationV1, digest: DefinitionDigest) -> RenewalOperationV1 {
        let mut row = row.clone();
        row.offer_digest = digest;
        row
    }
    fn with_holder_basis(
        row: &RenewalOperationV1,
        holder: Holder,
        basis: mush_economy::managed_paid_resale::RenewalHolderBasisV1,
    ) -> RenewalOperationV1 {
        let mut row = row.clone();
        row.binding.format = 2;
        row.binding.holder = holder;
        row.binding.holder_basis = Some(basis);
        row
    }
    fn decode(bytes: &[u8]) -> Result<RenewalOperationV1, PublicationError> {
        RenewalOperationV1::decode(bytes)
    }
    fn offer(
        row: &RenewalOperationV1,
    ) -> TestResult<mush_economy::managed_house_renewal::RenewalOffer> {
        Ok(ct0_snapshot::ct0_offer(row).map_err(|e| e.to_string())?)
    }
    fn off_side(
        mut offer: mush_economy::managed_house_renewal::RenewalOffer,
    ) -> mush_economy::managed_house_renewal::RenewalOffer {
        offer.terms.scope = mush_org::OrgScopeId(5);
        offer
    }
}

/// 17g C2g: `pub(crate)` with the helpers below, for Roll's command-read
/// tests (the named buyer's resale step on a granted licence).
pub(crate) type Birth = twin::Birth<House>;
#[allow(dead_code)]
fn definition(kind: &str, id: &str) -> TestResult<DefinitionRef> {
    twin::definition(kind, id)
}
#[allow(dead_code)]
fn identity() -> TestResult<RevisionIdentity> {
    twin::identity()
}
fn birth() -> TestResult<Birth> {
    twin::birth::<House>()
}
#[allow(dead_code)]
pub(crate) fn birth_with(terms: PaidTerms) -> TestResult<Birth> {
    twin::birth_with::<House>(terms)
}
pub(crate) fn draft(
    b: &Birth,
    n: u8,
    at: i64,
    attempt: RenewalAttemptV1,
) -> TestResult<RenewalOperationV1> {
    twin::draft::<House>(b, n, at, attempt)
}
#[allow(dead_code)]
pub(crate) fn funded(row: &RenewalOperationV1) -> TestResult<RenewalOperationV1> {
    twin::funded::<House>(row)
}
#[allow(dead_code)]
pub(crate) fn terminal(row: &RenewalOperationV1, grant: bool) -> TestResult<RenewalOperationV1> {
    twin::terminal::<House>(row, grant)
}
pub(crate) fn granted_birth(row: &RenewalOperationV1) -> TestResult<Birth> {
    twin::granted_birth::<House>(row)
}

super::super::renewal_twin::tests::instantiate!(crate::commerce::renewal_twin::House);

/// Doc 46 CT.5: every identity constant the twin reads for the house issuer, against the
/// literal its family's files held, and the family's old names against the constants. The
/// action ids, protocol names and offer domain are Economy's constants of the same issuer.
#[test]
fn house_identity_strings_hold_todays_bytes() -> TestResult {
    check_eq!(
        <House as crate::commerce::renewal_twin::Issuer>::OPERATION_FAMILY,
        "roll_house_mind_renewal_operation"
    );
    check_eq!(
        <House as crate::commerce::renewal_twin::Issuer>::HEAD_FAMILY,
        "roll_house_mind_renewal_head"
    );
    check_eq!(
        <House as crate::commerce::renewal_twin::Issuer>::PREDECESSOR_DOMAIN,
        "roll.mind.house_renewal_predecessor.v1"
    );
    check_eq!(
        <House as crate::commerce::renewal_twin::Issuer>::TEMPLATE_DOMAIN,
        "roll.mind.house_renewal_template.v1"
    );
    check_eq!(
        <House as mush_economy::renewal_twin::Issuer>::DRAFT_ACTION,
        "draft_house_mind_renewal"
    );
    check_eq!(
        <House as mush_economy::renewal_twin::Issuer>::FUND_ACTION,
        "fund_house_mind_renewal"
    );
    check_eq!(
        <House as mush_economy::renewal_twin::Issuer>::GRANT_ACTION,
        "settle_house_mind_renewal"
    );
    check_eq!(
        <House as mush_economy::renewal_twin::Issuer>::REFUND_ACTION,
        "refund_house_mind_renewal"
    );
    check_eq!(
        <House as mush_economy::renewal_twin::Issuer>::CLOSE_ACTION,
        "close_house_mind_renewal"
    );
    check_eq!(
        <House as mush_economy::renewal_twin::Issuer>::SERVICE_CLOSE_ACTION,
        "close_house_mind_renewal_service"
    );
    check_eq!(
        <House as mush_economy::renewal_twin::Issuer>::PROTOCOL,
        "house_mind_renewal_v1"
    );
    check_eq!(
        <House as mush_economy::renewal_twin::Issuer>::SERVICE_PROTOCOL,
        "house_mind_renewal_service_v1"
    );
    check_eq!(
        <House as mush_economy::renewal_twin::Issuer>::OFFER_DOMAIN,
        "economy.house_mind_renewal_offer.v1"
    );
    check_eq!(mush_economy::renewal_twin::MAX_RENEWAL_RECEIPT_BYTES, 1024);
    check_eq!(
        RenewalOperationV1::FAMILY,
        "roll_house_mind_renewal_operation"
    );
    check_eq!(RenewalHeadV1::FAMILY, "roll_house_mind_renewal_head");
    let ids: Vec<&str> = protocol::Transition::ALL.iter().map(|t| t.id()).collect();
    check_eq!(
        ids,
        [
            "draft_house_mind_renewal",
            "fund_house_mind_renewal",
            "settle_house_mind_renewal",
            "refund_house_mind_renewal",
            "close_house_mind_renewal"
        ]
    );
    check_eq!(
        <House as crate::commerce::renewal_twin::Rules>::anchor(token(1)).record(),
        format!("v1/house_renewal_operation/{}", token(1))
    );
    Ok(())
}

mod ct0_snapshot;

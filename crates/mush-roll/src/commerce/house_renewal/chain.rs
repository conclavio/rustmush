//! Bounded immutable term/holder/attempt reconstruction. The caller proves
//! every actual native acceptance and holder row; these are structural joins.
//!
//! Doc 46 CT.5: written once in `renewal_twin::chain`; the House chain's line is the House
//! that issued the licence.
#![allow(dead_code)]
use super::super::{
    TermGeneration,
    house_catalogue::HouseIssuer,
    house_purchase::HouseTermEconomicsV1,
    paid::{PaidTermRecord, PersonalPaidLicence},
    records::PublicationError,
    renewal_twin::House,
};
use super::records::{RenewalHeadV1, RenewalOperationV1};
use std::collections::BTreeMap;

pub(crate) fn check_draft(
    licence: &PersonalPaidLicence,
    row: &RenewalOperationV1,
    head: Option<&RenewalHeadV1>,
    previous: Option<&RenewalOperationV1>,
) -> Result<(), PublicationError> {
    super::super::renewal_twin::chain::check_draft::<House>(licence, row, head, previous)
}

pub(crate) fn verify(
    licence: &PersonalPaidLicence,
    issuer: HouseIssuer,
    head: Option<&RenewalHeadV1>,
    operations: &[&RenewalOperationV1],
    terms: &BTreeMap<TermGeneration, &PaidTermRecord>,
    economics: &BTreeMap<TermGeneration, &HouseTermEconomicsV1>,
) -> Result<(), PublicationError> {
    super::super::renewal_twin::chain::verify::<House>(
        licence, issuer, head, operations, terms, economics,
    )
}

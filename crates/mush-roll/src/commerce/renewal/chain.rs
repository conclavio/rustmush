//! Bounded immutable term/holder/attempt reconstruction. The caller proves
//! every actual native acceptance and holder row; these are structural joins.
//!
//! Doc 46 CT.5: written once in `renewal_twin::chain`; the personal chain has no line.
#![allow(dead_code)]
use super::super::{
    TermGeneration,
    entitlement::PaidTermEconomicsV1,
    paid::{PaidTermRecord, PersonalPaidLicence},
    records::PublicationError,
    renewal_twin::Personal,
};
use super::records::{RenewalHeadV1, RenewalOperationV1};
use std::collections::BTreeMap;

pub(crate) fn check_draft(
    licence: &PersonalPaidLicence,
    row: &RenewalOperationV1,
    head: Option<&RenewalHeadV1>,
    previous: Option<&RenewalOperationV1>,
) -> Result<(), PublicationError> {
    super::super::renewal_twin::chain::check_draft::<Personal>(licence, row, head, previous)
}

pub(crate) fn verify(
    licence: &PersonalPaidLicence,
    head: Option<&RenewalHeadV1>,
    operations: &[&RenewalOperationV1],
    terms: &BTreeMap<TermGeneration, &PaidTermRecord>,
    economics: &BTreeMap<TermGeneration, &PaidTermEconomicsV1>,
) -> Result<(), PublicationError> {
    super::super::renewal_twin::chain::verify::<Personal>(
        licence,
        (),
        head,
        operations,
        terms,
        economics,
    )
}

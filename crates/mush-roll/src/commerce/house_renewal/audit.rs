//! Roll/E4 half of mandatory renewal completeness, at the House issuer. The Market
//! paired audit is an additional required owner gate before configured commerce can open.
//!
//! Doc 46 CT.5: written once in `renewal_twin::audit`.
use super::super::renewal_twin::House;
use super::super::{
    families::CommerceFamilies,
    rating::RatingFamilies,
    records::PublicationError,
    roots::{RequiredRoot, RootAnchor},
};
use crate::RollStore;
use mush_core::ModuleCtx;
use std::collections::BTreeMap;

/// Recheck a later current pointer for a historical birth allocation (`renewal_twin::audit`).
pub(crate) fn verify_current_pointer(
    ctx: &ModuleCtx<'_>,
    roll: &RollStore,
    f: &CommerceFamilies,
    licence: &super::super::paid::PersonalPaidLicence,
) -> Result<(), PublicationError> {
    super::super::renewal_twin::audit::verify_current_pointer::<House>(ctx, roll, f, licence)
}

pub(crate) fn required(
    row: &super::RenewalOperationV1,
) -> Result<[RequiredRoot; 2], PublicationError> {
    super::super::renewal_twin::audit::required::<House>(row)
}

/// Full bounded scans run even with zero renewal operations (`renewal_twin::audit`).
pub(crate) fn audit(
    ctx: &ModuleCtx<'_>,
    roll: &RollStore,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
    enabled: bool,
) -> Result<BTreeMap<RootAnchor, super::super::RevisionIdentity>, PublicationError> {
    super::super::renewal_twin::audit::audit::<House>(ctx, roll, f, ratings, enabled)
}

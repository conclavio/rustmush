//! Admission only: reserve the actual complete Roll operation envelope for
//! both bounded receipts (`renewal_twin::capacity`, doc 46 CT.5).
use super::super::{families::CommerceFamilies, records::PublicationError};

pub(crate) fn check(
    f: &CommerceFamilies,
    row: &super::RenewalOperationV1,
) -> Result<(), PublicationError> {
    super::super::renewal_twin::capacity::check::<super::super::renewal_twin::Personal>(f, row)
}

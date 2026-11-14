//! The decode helpers both issuers' terms use. The wire they read is each family's own
//! (`subject.rs`); these only say that an optional economic role must be written out.

use crate::managed_entitlement::{GrossRecipientCut, Recipient};
use serde::Deserialize;

pub(crate) fn required_option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
}

// New renewal wire requires explicit nulls in every optional economic role.
// The shared C structs and their historical wire remain unchanged.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GrossWire {
    permille: u16,
    #[serde(deserialize_with = "required_option")]
    recipient: Option<Recipient>,
}
impl From<GrossWire> for GrossRecipientCut {
    fn from(w: GrossWire) -> Self {
        Self {
            permille: w.permille,
            recipient: w.recipient,
        }
    }
}
pub(crate) fn gross_cut<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<GrossRecipientCut, D::Error> {
    GrossWire::deserialize(d).map(Into::into)
}
pub(crate) fn venue_cut<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<GrossRecipientCut>, D::Error> {
    Option::<GrossWire>::deserialize(d).map(|v| v.map(Into::into))
}

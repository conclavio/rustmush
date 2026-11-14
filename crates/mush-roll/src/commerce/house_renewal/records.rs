//! Strict shared wire and Roll records for the fixed seven-target protocol.
//! All constructors are structural; actual E4 output and current rights are
//! checked separately before any owner image or typed row may be published.
//!
//! Doc 46 CT.5: the House family's own records. The binding is its `checked!` record with
//! its own fields (the issuing House among them); its methods, the closure and the operation
//! are the twin's one text (`renewal_twin::records`), expanded here for the House issuer.

#[allow(unused_imports)]
use super::super::records::Record;
use super::super::{
    CharterId, FiledRevisionId, Holder, LicenceId, LineId, RevisionIdentity, TermGeneration,
    house_catalogue::HouseIssuer,
    house_purchase::{HouseGrantEconomicsV1, HouseTermEconomicsV1},
    paid::PaidTerms,
    renewal_twin::{
        House,
        records::{binding_records, operation_records, present_holder},
    },
};
use mush_core::checked;
use mush_core::{CommitToken, DefinitionDigest};
use mush_economy::managed_paid_resale::RenewalHolderBasisV1;

pub use super::super::renewal_twin::records::RenewalAttemptV1;
#[cfg(test)]
pub(crate) use super::super::renewal_twin::records::{
    MAX_HEAD_BYTES, MAX_OPERATION_BYTES, MAX_OPERATIONS,
};
/// The House issuer's renewal state.
pub type RenewalStateV1 = super::super::renewal_twin::records::RenewalStateV1<House>;
/// The House issuer's renewal head.
pub type RenewalHeadV1 = super::super::renewal_twin::records::RenewalHeadV1<House>;

checked!(pub struct RenewalBindingV1, BindingWire, pub(crate) fields, attrs on both, {
    format:u16,licence:LicenceId,holder:Holder,issuer:HouseIssuer,charter:CharterId,line:LineId,
    birth_operation:CommitToken,predecessor:TermGeneration,next:TermGeneration,
    predecessor_operation:CommitToken,predecessor_term:DefinitionDigest,
    predecessor_economics:DefinitionDigest,filed:FiledRevisionId,revision:RevisionIdentity,
    terms:PaidTerms,term_seconds:i64,grace_seconds:i64,grant:HouseGrantEconomicsV1,
    product_digest:DefinitionDigest,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "present_holder")]
    holder_basis:Option<RenewalHolderBasisV1>,
});
binding_records! {
    issuer: House,
    economics: HouseTermEconomicsV1,
    grant: HouseGrantEconomicsV1,
    extra: { issuer: HouseIssuer },
}
impl RenewalBindingV1 {
    pub const fn issuer(&self) -> HouseIssuer {
        self.issuer
    }
}

operation_records!(House);

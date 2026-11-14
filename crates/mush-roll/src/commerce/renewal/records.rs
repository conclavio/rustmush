//! Strict shared wire and Roll records for the fixed seven-target protocol.
//! All constructors are structural; actual E4 output and current rights are
//! checked separately before any owner image or typed row may be published.
//!
//! Doc 46 CT.5: the personal family's own records. The binding is its `checked!` record with
//! its own fields; its methods, the closure and the operation are the twin's one text
//! (`renewal_twin::records`), expanded here for the personal issuer.

#[allow(unused_imports)]
use super::super::records::Record;
use super::super::{
    CharterId, FiledRevisionId, Holder, LicenceId, LineId, RevisionIdentity, TermGeneration,
    entitlement::{GrantEconomicsV1, PaidTermEconomicsV1},
    paid::PaidTerms,
    renewal_twin::{
        Personal,
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
/// The personal issuer's renewal state.
pub type RenewalStateV1 = super::super::renewal_twin::records::RenewalStateV1<Personal>;
/// The personal issuer's renewal head.
pub type RenewalHeadV1 = super::super::renewal_twin::records::RenewalHeadV1<Personal>;

checked!(pub struct RenewalBindingV1, BindingWire, pub(crate) fields, attrs on both, {
    format:u16,licence:LicenceId,holder:Holder,charter:CharterId,line:LineId,
    birth_operation:CommitToken,predecessor:TermGeneration,next:TermGeneration,
    predecessor_operation:CommitToken,predecessor_term:DefinitionDigest,
    predecessor_economics:DefinitionDigest,filed:FiledRevisionId,revision:RevisionIdentity,
    terms:PaidTerms,term_seconds:i64,grace_seconds:i64,grant:GrantEconomicsV1,
    product_digest:DefinitionDigest,
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "present_holder")]
    holder_basis:Option<RenewalHolderBasisV1>,
});
binding_records! {
    issuer: Personal,
    economics: PaidTermEconomicsV1,
    grant: GrantEconomicsV1,
    extra: {},
}

operation_records!(Personal);

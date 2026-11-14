//! Closed renewal evidence. Decoded values convey no entitlement or payment.
//!
//! Historical birth, immutable granted generations, and the current pointer
//! have separate checks. Only the paired owner writer can advance the pointer.
//!
//! Doc 46 CT.5: the personal instantiation. The Roll's renewal owner is written once for
//! both issuers in `commerce::renewal_twin`; this module is the personal issuer's
//! implementation of the twin's traits (its identity strings, its records, and the rules
//! only it has: the distinct seller pairing, the lineage and royalty parents of its offers,
//! the inverted House birth check of the audit), its own reader, and every path its callers
//! name, now aliases of the twin's types at the personal issuer.

pub(crate) mod audit;
mod chain;
pub(crate) mod history;
pub mod protocol;
pub mod reader;
mod records;
pub(crate) mod roots;

pub use records::{
    RenewalAttemptV1, RenewalBindingV1, RenewalClosureV1, RenewalHeadV1, RenewalOperationV1,
    RenewalStateV1,
};

#[cfg(test)]
pub(crate) mod tests;

pub mod writer;

#[cfg(test)]
mod capacity;

use super::{
    LicenceId,
    entitlement::{GrantEconomicsV1, PaidTermEconomicsV1, history::Birth},
    families::CommerceFamilies,
    paid::{PaidTerms, PersonalPaidLicence},
    participant::RootRequest,
    records::PublicationError,
    renewal_twin::{self, Issuer, Personal, Rules, roots::RenewalRoots},
    roots::RootAnchor,
};
use mush_core::participant::OwnerPrepareContext;
use mush_core::read_service::OwnerReadError;
use mush_core::{
    ActionTxn, CommitToken, DefinitionDigest, ModError, ModuleCtx, PrepareCtx, Registrar,
    StateFamilyHandle,
};
use mush_economy::{
    EconomyStore,
    managed_renewal::{RenewalLineageV1, RenewalOffer},
    renewal_twin::{RenewalAudit, RenewalAuditError},
};

impl Issuer for Personal {
    const OPERATION_FAMILY: &'static str = "roll_mind_renewal_operation";
    const HEAD_FAMILY: &'static str = "roll_mind_renewal_head";
    const PREDECESSOR_DOMAIN: &'static str = "roll.mind.renewal_predecessor.v1";
    const TEMPLATE_DOMAIN: &'static str = "roll.mind.renewal_template.v1";

    type Binding = RenewalBindingV1;
    type Operation = RenewalOperationV1;
    type Closure = RenewalClosureV1;
    type Economics = PaidTermEconomicsV1;
    type Grant = GrantEconomicsV1;
    type Reader = reader::RenewalReader;
    type Facts = reader::RenewalFacts;
    type Admission = ();
    type Line = ();
}

impl Rules for Personal {
    type StoredOperation = RenewalOperationV1;
    type StoredEconomics = PaidTermEconomicsV1;

    fn operations(f: &CommerceFamilies) -> &StateFamilyHandle<String, RenewalOperationV1> {
        &f.renewal_operations
    }
    fn heads(f: &CommerceFamilies) -> &StateFamilyHandle<String, RenewalHeadV1> {
        &f.renewal_heads
    }
    fn economics(f: &CommerceFamilies) -> &StateFamilyHandle<String, PaidTermEconomicsV1> {
        &f.term_economics
    }
    fn admission_capacity(
        family: &StateFamilyHandle<String, PaidTermEconomicsV1>,
        row: &PaidTermEconomicsV1,
    ) -> Result<(), PublicationError> {
        super::entitlement::admission_capacity::check(family, row)
    }
    fn audit(economy: &EconomyStore) -> Result<RenewalAudit<'_, Personal>, RenewalAuditError> {
        economy.renewal_audit()
    }
    fn require_no_history(economy: &EconomyStore) -> Result<(), RenewalAuditError> {
        economy.require_no_renewal_history()
    }
    fn anchor(operation: CommitToken) -> RootAnchor {
        RootAnchor::RenewalOperation(operation)
    }
    fn root_request(roots: RenewalRoots<Personal>) -> RootRequest {
        RootRequest::Renewal(Box::new(roots))
    }
    fn own(licence: LicenceId) -> reader::RenewalRequest {
        reader::RenewalRequest::own(licence)
    }
    fn into_facts(read: reader::RenewalRead) -> Result<reader::RenewalFacts, OwnerReadError> {
        read.into_facts()
    }
    fn install_extra(_: &mut Registrar<'_>, _: &CommerceFamilies) -> Result<(), ModError> {
        Ok(())
    }
    fn observe_accounts(
        ctx: &mut PrepareCtx<'_>,
        facts: &reader::RenewalFacts,
        (): &(),
    ) -> Result<(), PublicationError> {
        let pair = ctx
            .distinct_personal_accounts(facts.sale().seller())
            .map_err(|_| PublicationError::Mismatch)?;
        if pair.requester() != facts.sale().requester()
            || facts.binding().holder().acting_character() != pair.requester()
            || pair.other() != facts.sale().seller()
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
    fn next_present(
        _: &mut ActionTxn<'_>,
        _: &CommerceFamilies,
        _: &String,
    ) -> Result<bool, PublicationError> {
        Ok(false)
    }
    fn offer_differs_from_sale(offer: &RenewalOffer, facts: &reader::RenewalFacts) -> bool {
        let sale = facts.sale();
        let terms = &offer.terms;
        let lineage = if sale.parent().is_some() {
            RenewalLineageV1::Derived
        } else {
            RenewalLineageV1::Original
        };
        terms.lineage != lineage
            || !terms.buyer.same_principal(facts.binding().holder())
            || terms.buyer.acting_character() != sale.requester()
            || terms.seller != sale.seller()
            || terms.title_revision != sale.title_revision()
            || terms.parents.len() != sale.royalty().edges().len()
            || terms
                .parents
                .iter()
                .zip(sale.royalty().edges())
                .any(|(cut, edge)| {
                    cut.charter != mush_core::ObjId(edge.charter().get())
                        || cut.permille != edge.royalty_permille()
                })
    }
    fn offer_differs_from_binding(binding: &RenewalBindingV1, offer: &RenewalOffer) -> bool {
        offer.basis.royalty_chain_digest != binding.grant().royalty_chain().digest()
    }
    fn admit_fresh(
        (): &(),
        _: &OwnerPrepareContext<'_>,
        _: &reader::RenewalFacts,
    ) -> Result<(), PublicationError> {
        Ok(())
    }
    fn line(
        _: &ModuleCtx<'_>,
        _: &CommerceFamilies,
        _: &PersonalPaidLicence,
    ) -> Result<(), PublicationError> {
        Ok(())
    }
    fn off_line(_: &RenewalBindingV1, (): ()) -> bool {
        false
    }
    fn check_birth(
        birth: &Birth<'_>,
        binding: &RenewalBindingV1,
        offer: &RenewalOffer,
    ) -> Result<(), PublicationError> {
        let Birth::Entitlement(birth) = birth else {
            return Err(PublicationError::Mismatch);
        };
        if binding.product_digest() != birth.product_digest
            || binding.grant() != birth.economics.grant()
            || offer.basis.prior_quote != birth.quote
            || offer.basis.prior_quote_digest != birth.quote_digest
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
    fn other_issuers_birth(birth: &Birth<'_>) -> bool {
        // The disjoint House audit proves every actual House generation.
        matches!(birth, Birth::House(_))
    }
    fn seats_differ(terms: PaidTerms, _: &GrantEconomicsV1) -> Result<bool, PublicationError> {
        Ok(terms.seats().get() != 1)
    }
}

impl renewal_twin::Facts<Personal> for reader::RenewalFacts {
    fn binding(&self) -> &RenewalBindingV1 {
        reader::RenewalFacts::binding(self)
    }
    fn prior_quote(&self) -> CommitToken {
        reader::RenewalFacts::prior_quote(self)
    }
    fn prior_quote_digest(&self) -> DefinitionDigest {
        reader::RenewalFacts::prior_quote_digest(self)
    }
    fn check_fresh(
        &self,
        ctx: &OwnerPrepareContext<'_>,
        amount: Option<i64>,
    ) -> Result<(), PublicationError> {
        reader::RenewalFacts::check_fresh(self, ctx, amount)
    }
}

impl renewal_twin::TermEconomics for PaidTermEconomicsV1 {
    type Grant = GrantEconomicsV1;
    fn licence(&self) -> LicenceId {
        PaidTermEconomicsV1::licence(self)
    }
    fn generation(&self) -> super::TermGeneration {
        PaidTermEconomicsV1::generation(self)
    }
    fn operation(&self) -> CommitToken {
        PaidTermEconomicsV1::operation(self)
    }
}

//! Closed renewal of an actually issued personal House licence.
//! Birth, current pointer and retained terminal histories are distinct proofs.
//!
//! Doc 46 CT.5: the House instantiation. The Roll's renewal owner is written once for both
//! issuers in `commerce::renewal_twin`; this module is the House issuer's implementation of
//! the twin's traits (its identity strings, its records, and the rules only it has: the
//! requester's own account, the purchase composition's buyer admission, the organisation and
//! scope of its offers, the House a chain is held to), its own reader, and every path its
//! callers name, now aliases of the twin's types at the House issuer.
mod chain;
pub(crate) mod history;
pub mod protocol;
mod records;
pub use records::{
    RenewalAttemptV1, RenewalBindingV1, RenewalClosureV1, RenewalHeadV1, RenewalOperationV1,
    RenewalStateV1,
};

pub(crate) mod audit;
#[cfg(test)]
mod capacity;
pub mod reader;
pub(crate) mod roots;
pub mod writer;

#[cfg(test)]
mod tests;

use super::{
    LicenceId,
    entitlement::history::{Birth, resolve},
    families::CommerceFamilies,
    house_catalogue::HouseIssuer,
    house_purchase::{
        HouseBuyerAdmission, HouseGrantEconomicsV1, HouseProtocol, HouseTermEconomicsV1,
    },
    paid::{PaidTerms, PersonalPaidLicence},
    participant::RootRequest,
    records::PublicationError,
    renewal_twin::{self, House, Issuer, Rules, roots::RenewalRoots},
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
    managed_house_renewal::RenewalOffer,
    renewal_twin::{RenewalAudit, RenewalAuditError},
};
use std::sync::Arc;

impl Issuer for House {
    const OPERATION_FAMILY: &'static str = "roll_house_mind_renewal_operation";
    const HEAD_FAMILY: &'static str = "roll_house_mind_renewal_head";
    const PREDECESSOR_DOMAIN: &'static str = "roll.mind.house_renewal_predecessor.v1";
    const TEMPLATE_DOMAIN: &'static str = "roll.mind.house_renewal_template.v1";

    type Binding = RenewalBindingV1;
    type Operation = RenewalOperationV1;
    type Closure = RenewalClosureV1;
    type Economics = HouseTermEconomicsV1;
    type Grant = HouseGrantEconomicsV1;
    type Reader = reader::RenewalReader;
    type Facts = reader::RenewalFacts;
    type Admission = Option<Arc<dyn HouseBuyerAdmission>>;
    type Line = HouseIssuer;
}

impl Rules for House {
    type StoredOperation = RenewalOperationV1;
    type StoredEconomics = HouseTermEconomicsV1;

    fn operations(f: &CommerceFamilies) -> &StateFamilyHandle<String, RenewalOperationV1> {
        &f.house_renewal_operations
    }
    fn heads(f: &CommerceFamilies) -> &StateFamilyHandle<String, RenewalHeadV1> {
        &f.house_renewal_heads
    }
    fn economics(f: &CommerceFamilies) -> &StateFamilyHandle<String, HouseTermEconomicsV1> {
        &f.house_economics
    }
    fn admission_capacity(
        family: &StateFamilyHandle<String, HouseTermEconomicsV1>,
        row: &HouseTermEconomicsV1,
    ) -> Result<(), PublicationError> {
        super::house_purchase::admission_capacity::check(family, row)
    }
    fn audit(economy: &EconomyStore) -> Result<RenewalAudit<'_, House>, RenewalAuditError> {
        economy.house_renewal_audit()
    }
    fn require_no_history(economy: &EconomyStore) -> Result<(), RenewalAuditError> {
        economy.require_no_house_renewal_history()
    }
    fn anchor(operation: CommitToken) -> RootAnchor {
        RootAnchor::HouseRenewalOperation(operation)
    }
    fn root_request(roots: RenewalRoots<House>) -> RootRequest {
        RootRequest::HouseRenewal(Box::new(roots))
    }
    fn own(licence: LicenceId) -> reader::RenewalRequest {
        reader::RenewalRequest::own(licence)
    }
    fn into_facts(read: reader::RenewalRead) -> Result<reader::RenewalFacts, OwnerReadError> {
        read.into_facts()
    }
    fn install_extra(reg: &mut Registrar<'_>, families: &CommerceFamilies) -> Result<(), ModError> {
        // `absent_next` proves, inside Market's transaction, that the next
        // generation has no entitlement economics yet; Market only reads it
        // (a House-only composition delegates it nowhere): a read-only grant.
        reg.grant_state_family_read(&families.term_economics, "market")
    }
    fn observe_accounts(
        ctx: &mut PrepareCtx<'_>,
        facts: &reader::RenewalFacts,
        admission: &Option<Arc<dyn HouseBuyerAdmission>>,
    ) -> Result<(), PublicationError> {
        let account = ctx
            .personal_account()
            .map_err(|_| PublicationError::Mismatch)?;
        if account.requester() != facts.sale().requester()
            || facts.binding().holder().acting_character() != account.requester()
        {
            return Err(PublicationError::Mismatch);
        }
        if let Some(policy) = admission {
            policy.check(
                ctx.world().map_err(|_| PublicationError::Mismatch)?,
                facts.sale().issuer(),
                account.requester(),
            )?;
        }
        Ok(())
    }
    fn next_present(
        txn: &mut ActionTxn<'_>,
        f: &CommerceFamilies,
        key: &String,
    ) -> Result<bool, PublicationError> {
        Ok(txn
            .get(&f.house_economics, key)
            .map_err(|_| PublicationError::Mismatch)?
            .is_some())
    }
    fn offer_differs_from_sale(offer: &RenewalOffer, facts: &reader::RenewalFacts) -> bool {
        let sale = facts.sale();
        let terms = &offer.terms;
        !terms.buyer.same_principal(facts.binding().holder())
            || terms.buyer.acting_character() != sale.requester()
            || terms.organization != sale.issuer().organization()
            || terms.scope != sale.issuer().scope()
            || terms.title_revision != sale.title_revision()
    }
    fn offer_differs_from_binding(binding: &RenewalBindingV1, offer: &RenewalOffer) -> bool {
        offer.terms.organization != binding.issuer().organization()
            || offer.terms.scope != binding.issuer().scope()
    }
    fn admit_fresh(
        admission: &Option<Arc<dyn HouseBuyerAdmission>>,
        ctx: &OwnerPrepareContext<'_>,
        facts: &reader::RenewalFacts,
    ) -> Result<(), PublicationError> {
        if let Some(policy) = admission {
            policy.check(ctx.world(), facts.sale().issuer(), ctx.invocation().actor())?;
        }
        Ok(())
    }
    fn line(
        ctx: &ModuleCtx<'_>,
        f: &CommerceFamilies,
        licence: &PersonalPaidLicence,
    ) -> Result<HouseIssuer, PublicationError> {
        let Birth::House(birth_row) = resolve(ctx, f, licence.operation())? else {
            return Err(PublicationError::Mismatch);
        };
        Ok(birth_row.issuer)
    }
    fn off_line(binding: &RenewalBindingV1, line: HouseIssuer) -> bool {
        binding.issuer() != line
    }
    fn check_birth(
        birth: &Birth<'_>,
        binding: &RenewalBindingV1,
        offer: &RenewalOffer,
    ) -> Result<(), PublicationError> {
        let Birth::House(birth) = birth else {
            return Err(PublicationError::Mismatch);
        };
        if binding.issuer() != birth.issuer
            || binding.product_digest() != birth.product_digest
            || binding.grant() != birth.economics.grant()
            || offer.basis.prior_quote != birth.quote
            || offer.basis.prior_quote_digest != birth.quote_digest
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
    fn other_issuers_birth(birth: &Birth<'_>) -> bool {
        !matches!(birth, Birth::House(_))
    }
    fn seats_differ(
        terms: PaidTerms,
        grant: &HouseGrantEconomicsV1,
    ) -> Result<bool, PublicationError> {
        Ok(terms.seats().get() != HouseProtocol::from_format(grant.renewal().format())?.seats())
    }
}

impl renewal_twin::Facts<House> for reader::RenewalFacts {
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

impl renewal_twin::TermEconomics for HouseTermEconomicsV1 {
    type Grant = HouseGrantEconomicsV1;
    fn licence(&self) -> LicenceId {
        HouseTermEconomicsV1::licence(self)
    }
    fn generation(&self) -> super::TermGeneration {
        HouseTermEconomicsV1::generation(self)
    }
    fn operation(&self) -> CommitToken {
        HouseTermEconomicsV1::operation(self)
    }
}

impl writer::RenewalWriter {
    /// Carry the actual purchase composition's refusal-only policy into fresh
    /// next-term admission. Refund/Close never evaluate current buyer eligibility.
    pub fn with_purchase_admission(
        mut self,
        purchase: &super::house_purchase::writer::HousePurchaseWriter,
    ) -> Result<Self, ModError> {
        if self.0.get().is_some() || self.1.is_some() {
            return Err(ModError::msg("renewal buyer policy already configured"));
        }
        self.1 = purchase.buyer_admission();
        Ok(self)
    }
}

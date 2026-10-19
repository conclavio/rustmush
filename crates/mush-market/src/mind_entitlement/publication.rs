//! Closed record derivation from actual prepared E4 and Roll outputs. These
//! methods do not install rows and cannot manufacture an economic receipt.

use mush_core::DefinitionRef;
use mush_economy::managed_entitlement::{EntitlementFundingOutput, EntitlementTerminalOutput};
use mush_economy::managed_purchase::{PurchaseHistory, PurchaseOutcome};
use mush_roll::commerce::entitlement::{EntitlementAllocationV1, writer::EntitlementPurchaseTerms};
use mush_roll::commerce::purchase::{PurchaseParticipants, PurchasePhase};

use super::codec::Row;
use super::purchase::{EntitlementPurchaseV1, EntitlementReceiptV1, Outcome};
use super::records::{EntitlementQuoteV1, EntitlementSubjectV1};

fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}
impl EntitlementSubjectV1 {
    pub(super) fn capture(
        f: &mush_roll::commerce::entitlement::reader::EntitlementSaleFacts,
    ) -> Self {
        Self {
            seller: f.seller(),
            title_revision: f.title_revision(),
            charter: f.charter(),
            line: f.line(),
            filed: f.filed(),
            sequence: f.sequence(),
            owner_licence: f.owner_licence(),
            owner_term: f.owner_term(),
            owner_window: f.owner_window(),
            revision: f.revision(),
            schema: f.schema(),
            definition: f.definition(),
            rating: f.rating(),
            compatibility: f.compatibility().digest(),
            declared: f.declared(),
            minimum: f.minimum(),
            parent: f.parent(),
        }
    }
}
pub(super) fn terms(q: &EntitlementQuoteV1) -> EntitlementPurchaseTerms {
    EntitlementPurchaseTerms {
        terms: q.product.terms,
        term_seconds: q.product.term_seconds,
        grace_seconds: q.product.grace_seconds,
        grant: q.product.grant.clone(),
    }
}
impl EntitlementPurchaseV1 {
    pub(super) fn funded(
        q: EntitlementQuoteV1,
        effect: DefinitionRef,
        participants: PurchaseParticipants,
        output: &EntitlementFundingOutput,
        allocation: &EntitlementAllocationV1,
    ) -> Result<Self, String> {
        q.validate()?;
        let economic = q.economic_subject()?;
        if output.marker().operation() != output.operation()
            || output.marker().effect() != &effect
            || output.marker().entitlement_protocol() != Some(economic.protocol)
            || output.marker().economic_digest() != Some(economic.economic_digest().map_err(error)?)
            || output.marker().quote() != q.publication
            || output.marker().quote_digest() != q.digest()?
            || output.marker().product_digest() != q.product_digest
        {
            return Err("entitlement funding output differs from acknowledged consent".into());
        }
        let row = Self {
            format: 1,
            operation: output.operation(),
            effect,
            participants,
            quote_digest: q.digest()?,
            quote: q,
            licence: allocation.licence(),
            term: allocation.term(),
            funding: output.receipt().clone(),
            phase: PurchasePhase::Funded {},
            updated_by: output.operation(),
        };
        row.validate()?;
        row.matches_allocation(allocation)?;
        Ok(row)
    }
    pub(super) fn terminal(
        &self,
        output: &EntitlementTerminalOutput,
    ) -> Result<(Self, EntitlementReceiptV1), String> {
        self.validate()?;
        if self.phase != (PurchasePhase::Funded {})
            || output.operation() != self.operation
            || output.contract() != self.funding.contract()
            || output.hold() != self.funding.hold()
        {
            return Err("entitlement terminal output differs from funded custody".into());
        }
        let mut after = self.clone();
        after.updated_by = output.commit();
        after.phase = match output.outcome() {
            PurchaseOutcome::Granted => PurchasePhase::Granted {
                commit: output.commit(),
            },
            PurchaseOutcome::Refunded => PurchasePhase::Refunded {
                commit: output.commit(),
            },
        };
        let receipt = EntitlementReceiptV1 {
            format: 1,
            operation: self.operation,
            commit: output.commit(),
            outcome: match output.outcome() {
                PurchaseOutcome::Granted => Outcome::Granted {},
                PurchaseOutcome::Refunded => Outcome::Refunded {},
            },
            contract: output.contract(),
            hold: output.hold(),
            transaction: output.transaction(),
            journal_seq: output.journal_seq(),
            licence: self.licence,
            term: self.term,
            quote_digest: self.quote_digest,
            product_digest: self.quote.product_digest,
            grant_economics_digest: self.quote.product.grant.digest().map_err(error)?,
            issued_at_ic: output.issued_at_ic().cloned(),
        };
        after.validate()?;
        receipt.matches_purchase(&after)?;
        Ok((after, receipt))
    }
    pub(super) fn verify_history(&self, history: &PurchaseHistory) -> Result<(), String> {
        self.validate()?;
        let economic = self.quote.economic_subject()?;
        if history.operation() != self.operation
            || history.effect() != &self.effect
            || !history.matches_entitlement(&economic, &self.funding)
            || history.entitlement().map_err(error)?.as_ref() != Some(&economic)
        {
            return Err("entitlement operation differs from actual E4 history".into());
        }
        let expected = match history.terminal() {
            None => PurchasePhase::Funded {},
            Some(t) if t.granted() => PurchasePhase::Granted { commit: t.commit() },
            Some(t) => PurchasePhase::Refunded { commit: t.commit() },
        };
        if self.phase != expected {
            return Err("entitlement terminal phase differs from actual E4 history".into());
        }
        Ok(())
    }
}
impl EntitlementReceiptV1 {
    pub(super) fn verify_history(
        &self,
        row: &EntitlementPurchaseV1,
        history: &PurchaseHistory,
    ) -> Result<(), String> {
        row.verify_history(history)?;
        self.matches_purchase(row)?;
        let terminal = history
            .terminal()
            .ok_or("entitlement terminal history is missing")?;
        let outcome = if terminal.granted() {
            Outcome::Granted {}
        } else {
            Outcome::Refunded {}
        };
        if self.commit != terminal.commit()
            || self.outcome != outcome
            || self.transaction != terminal.transaction()
            || self.journal_seq != terminal.journal_seq()
            || self.issued_at_ic.as_ref() != terminal.issued_at_ic()
        {
            return Err("entitlement receipt differs from actual E4 terminal".into());
        }
        Ok(())
    }
}

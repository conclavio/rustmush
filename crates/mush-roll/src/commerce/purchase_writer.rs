//! Closed cross-owner write bridge. Only Roll registration installs its real
//! family handles, and only Market registration receives action-bound methods.
//! No caller can obtain a raw family or retention participant handle here.

use std::sync::{Arc, OnceLock};

use mush_core::action_state::ActionTxn;
use mush_core::participant::ParticipantHandle;
use mush_core::read_service::OwnerReadHandle;
use mush_core::{CommitToken, DefinitionDigest, DefinitionRef, ModError, PrepareCtx, Registrar};
use mush_economy::managed_purchase::{FundingOutput, PurchaseOutcome, TerminalOutput};

use super::families::CommerceFamilies;
use super::paid::{PaidTermRecord, PaidTerms, PersonalPaidLicence};
use super::paid_roots::PaidRoots;
use super::participant::{RootParticipant, RootRequest};
use super::purchase::{PaidAllocation, PurchaseParticipants, PurchasePhase};
use super::rating::RatingClass;
use super::records::{PublicationError, Record, RevisionPolicy};
use super::sale_reader::{OriginalSaleFacts, OriginalSaleReader, OriginalSaleRequest};
use super::{LicenceId, TermGeneration, TermWindow, WorldIcSeconds};

/// Frozen data derived by Market from its exact acknowledged quote. These
/// values authorize nothing by themselves; the complete Market post-images
/// must also pass the closed core purchase/terminal-capacity verifier.
#[derive(Clone, Debug)]
pub struct PurchaseQuoteIdentity {
    pub quote: CommitToken,
    pub quote_digest: DefinitionDigest,
    pub product_digest: DefinitionDigest,
}
#[derive(Clone, Copy, Debug)]
pub struct OriginalPurchaseTerms {
    pub class: RatingClass,
    pub form: super::paid::PaidForm,
    pub transferable: bool,
    pub term_seconds: i64,
    pub grace_seconds: i64,
}
impl OriginalPurchaseTerms {
    fn terms(self) -> Result<PaidTerms, PublicationError> {
        if !(1..=super::MAX_TERM_SECONDS).contains(&self.term_seconds)
            || !(0..=super::MAX_GRACE_SECONDS).contains(&self.grace_seconds)
        {
            return Err(PublicationError::Invalid);
        }
        if self.form == (super::paid::PaidForm::Sealed {}) {
            PaidTerms::original_personal(self.class, self.transferable)
        } else {
            PaidTerms::original_personal_form(self.class, self.form, self.transferable)
        }
    }
    fn window(self, stamp: &mush_core::ClockStamp) -> Result<TermWindow, PublicationError> {
        self.terms()?;
        let issued = WorldIcSeconds::from_stamp(stamp).map_err(invalid)?;
        let mut end = stamp.clone();
        end.at = end
            .at
            .checked_add(self.term_seconds)
            .ok_or(PublicationError::Capacity)?;
        let term_end = WorldIcSeconds::from_stamp(&end).map_err(invalid)?;
        end.at = end
            .at
            .checked_add(self.grace_seconds)
            .ok_or(PublicationError::Capacity)?;
        TermWindow::new(
            issued,
            issued,
            term_end,
            WorldIcSeconds::from_stamp(&end).map_err(invalid)?,
        )
        .map_err(invalid)
    }
}

#[derive(Clone)]
struct Registration {
    families: CommerceFamilies,
    manifest: DefinitionRef,
    participants: PurchaseParticipants,
    actions: [DefinitionRef; 3],
}

/// Trusted composition shares this one empty cell between Roll and Market.
/// Roll installs it exactly once before the final definition freeze. Market
/// may register earlier; its closed binding validator checks this exact cell.
/// There is no runtime setter or externally constructible registered payload.
#[derive(Clone, Default)]
pub struct OriginalPurchaseWriter(Arc<OnceLock<Registration>>);
impl std::fmt::Debug for OriginalPurchaseWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OriginalPurchaseWriter")
            .field("registered", &self.0.get().is_some())
            .finish()
    }
}
impl OriginalPurchaseWriter {
    pub fn new() -> Self {
        Self::default()
    }
    pub(crate) fn install(
        &self,
        reg: &mut Registrar<'_>,
        families: CommerceFamilies,
        manifest: DefinitionRef,
        participants: PurchaseParticipants,
        actions: [DefinitionRef; 3],
    ) -> Result<(), ModError> {
        if reg.module() != crate::NS || self.0.get().is_some() {
            return Err(ModError::msg("paid writer must be installed once by Roll"));
        }
        reg.delegate_state_family(&families.deployment, "market")?;
        reg.delegate_state_family(&families.paid_allocations, "market")?;
        reg.delegate_state_family(&families.paid_licences, "market")?;
        reg.delegate_state_family(&families.paid_terms, "market")?;
        self.0
            .set(Registration {
                families,
                manifest,
                participants,
                actions,
            })
            .map_err(|_| ModError::msg("paid writer registration conflicts"))
    }

    /// Closed Market boot join over the exact privately delegated families.
    /// This reads retained data only; no current eligibility or grant is minted.
    pub fn audit_allocation_for_boot(
        &self,
        ctx: &mush_core::ModuleCtx<'_>,
        operation: CommitToken,
        terms: OriginalPurchaseTerms,
        issued: Option<&mush_core::ClockStamp>,
    ) -> Result<PaidAllocation, PublicationError> {
        if ctx.module.as_deref() != Some("market") {
            return Err(PublicationError::Unavailable);
        }
        ctx.state::<crate::RollStore>()
            .ok_or(PublicationError::Unavailable)?
            .commerce_ready()
            .map_err(invalid)?;
        let r = self.0.get().ok_or(PublicationError::Unavailable)?;
        let row = super::audit::read(ctx, &r.families.paid_allocations, operation.to_hex())?;
        let roll = ctx
            .state::<crate::RollStore>()
            .ok_or(PublicationError::Unavailable)?;
        for root in super::paid_roots::required(row)? {
            roll.check_required_root(&root).map_err(invalid)?;
        }
        if row.terms != terms.terms()?
            || matches!(row.phase, PurchasePhase::Granted { .. }) != issued.is_some()
        {
            return Err(PublicationError::Mismatch);
        }
        let licence = ctx
            .action_state(&r.families.paid_licences, &row.licence.key())
            .map_err(invalid)?;
        let term = ctx
            .action_state(
                &r.families.paid_terms,
                &format!("{}/{}", row.licence.key(), row.term.key()),
            )
            .map_err(invalid)?;
        if let Some(stamp) = issued {
            let expected_licence = PersonalPaidLicence::original(
                row.licence,
                row.charter,
                row.line,
                row.holder,
                row.term,
                row.operation,
            )?;
            let expected_term = PaidTermRecord::original(
                row.licence,
                row.term,
                terms.window(stamp)?,
                row.terms,
                RevisionPolicy::Pin {
                    filed: row.filed,
                    revision: row.revision,
                },
                row.operation,
            )?;
            if licence != Some(&expected_licence) || term != Some(&expected_term) {
                return Err(PublicationError::Mismatch);
            }
        } else if licence.is_some() || term.is_some() {
            return Err(PublicationError::Mismatch);
        }
        Ok(row.clone())
    }

    /// Final composition check for Market's closed binding definition. This is
    /// read-only; only the actual Roll registrar can install private handles.
    pub fn validate_registration(
        &self,
        manifest: &DefinitionRef,
        participants: &PurchaseParticipants,
        actions: &[DefinitionRef; 3],
    ) -> Result<(), ModError> {
        let registration = self
            .0
            .get()
            .ok_or_else(|| ModError::msg("Roll paid writer is not registered"))?;
        if manifest != &registration.manifest
            || participants != &registration.participants
            || actions != &registration.actions
        {
            return Err(ModError::msg("paid writer composition differs"));
        }
        Ok(())
    }

    /// Create a closed action-bound reference during either owner order. The
    /// final Market definition validator rejects missing or mismatched wiring
    /// after all registrations. Every runtime method also checks the exact cell.
    /// Reuse Market's already granted reader; core still checks its actual
    /// composition, requester and service definition on every read.
    pub fn for_action(
        &self,
        reg: &mut Registrar<'_>,
        action: &DefinitionRef,
        manifest: &DefinitionRef,
        participants: &PurchaseParticipants,
        reader: OwnerReadHandle<OriginalSaleReader>,
    ) -> Result<RegisteredPurchaseWriter, ModError> {
        if reg.module() != "market" {
            return Err(ModError::msg("paid writer requires Market registration"));
        }
        let writer = RegisteredPurchaseWriter {
            action: action.clone(),
            registration: self.clone(),
            manifest: manifest.clone(),
            participants: participants.clone(),
            roots: reg.request_owner_participant::<RootParticipant>(action.clone())?,
            reader,
        };
        if self.0.get().is_some() {
            writer
                .registration()
                .map_err(|_| ModError::msg("paid writer composition differs"))?;
        }
        Ok(writer)
    }
}

pub struct RegisteredPurchaseWriter {
    action: DefinitionRef,
    registration: OriginalPurchaseWriter,
    manifest: DefinitionRef,
    participants: PurchaseParticipants,
    roots: ParticipantHandle<RootParticipant>,
    reader: OwnerReadHandle<OriginalSaleReader>,
}

fn invalid<T>(_: T) -> PublicationError {
    PublicationError::Invalid
}

impl RegisteredPurchaseWriter {
    fn registration(&self) -> Result<&Registration, PublicationError> {
        let registration = self
            .registration
            .0
            .get()
            .ok_or(PublicationError::Unavailable)?;
        if self.manifest != registration.manifest
            || self.participants != registration.participants
            || !registration.actions.contains(&self.action)
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(registration)
    }

    pub fn allocation(
        &self,
        txn: &mut ActionTxn<'_>,
        operation: CommitToken,
    ) -> Result<PaidAllocation, PublicationError> {
        let row = txn
            .get(
                &self.registration()?.families.paid_allocations,
                &operation.to_hex(),
            )
            .map_err(invalid)?
            .cloned()
            .ok_or(PublicationError::Missing)?;
        row.validate()?;
        if row.operation != operation {
            return Err(PublicationError::Mismatch);
        }
        Ok(row)
    }

    fn current(
        &self,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        facts: &OriginalSaleFacts,
    ) -> Result<(), PublicationError> {
        let actual = ctx
            .read_owner_preimage(
                txn,
                &self.reader,
                OriginalSaleRequest::listed(facts.seller(), facts.owner_licence()),
            )
            .map_err(invalid)?;
        if actual != *facts {
            return Err(PublicationError::Mismatch);
        }
        // The same account cannot buy its own alternate character's licence.
        // This proof is core-minted for the actual authenticated invocation.
        ctx.distinct_personal_accounts(facts.seller())
            .map_err(invalid)?;
        Ok(())
    }

    pub fn fund(
        &self,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        quote: PurchaseQuoteIdentity,
        terms: OriginalPurchaseTerms,
        facts: &OriginalSaleFacts,
        output: &FundingOutput,
    ) -> Result<PaidAllocation, PublicationError> {
        let commit = ctx.commit_token();
        if self.action.key().id.as_str() != "purchase_original_mind"
            || output.operation() != commit
            || output.marker().effect() != &self.action
            || output.marker().quote() != quote.quote
            || output.marker().quote_digest() != quote.quote_digest
            || output.marker().product_digest() != quote.product_digest
            || terms.class < facts.minimum()
            || !matches!(self.action.version(), 1 | 2)
            || (self.action.version() == 1 && terms.form != (super::paid::PaidForm::Sealed {}))
        {
            return Err(PublicationError::Mismatch);
        }
        self.current(ctx, txn, facts)?;
        let families = &self.registration()?.families;
        let mut deployment = txn
            .get(&families.deployment, &"registry".into())
            .map_err(invalid)?
            .cloned()
            .ok_or(PublicationError::Unavailable)?;
        deployment.validate()?;
        let licence = LicenceId::new(deployment.next_licence).map_err(invalid)?;
        let term = TermGeneration::new(1).map_err(invalid)?;
        deployment.next_licence = deployment
            .next_licence
            .checked_add(1)
            .ok_or(PublicationError::Capacity)?;
        let row = PaidAllocation {
            format: u16::try_from(self.action.version()).map_err(|_| PublicationError::Invalid)?,
            operation: commit,
            participants: self.registration()?.participants.clone(),
            licence,
            term,
            quote: quote.quote,
            quote_digest: quote.quote_digest,
            product_digest: quote.product_digest,
            holder: facts.requester(),
            charter: facts.charter(),
            line: facts.line(),
            filed: facts.filed(),
            revision: facts.revision(),
            rating: facts.rating(),
            terms: terms.terms()?,
            funding: output.receipt().clone(),
            phase: PurchasePhase::Funded {},
            updated_by: commit,
        };
        row.validate()?;
        let root = PaidRoots::funding(&row, commit, facts)?;
        txn.replace(&families.deployment, &"registry".into(), deployment)
            .map_err(invalid)?;
        txn.insert(&families.paid_allocations, &row.key(), row.clone())
            .map_err(invalid)?;
        if txn
            .get(&families.paid_allocations, &row.key())
            .map_err(invalid)?
            != Some(&row)
        {
            return Err(PublicationError::Mismatch);
        }
        ctx.prepare_owner(&self.roots, RootRequest::Paid(Box::new(root)))
            .map_err(invalid)?;
        Ok(row)
    }

    pub fn terminal(
        &self,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        before: &PaidAllocation,
        terms: OriginalPurchaseTerms,
        facts: Option<&OriginalSaleFacts>,
        output: &TerminalOutput,
    ) -> Result<PaidAllocation, PublicationError> {
        if self.allocation(txn, before.operation)? != *before
            || output.commit() != ctx.commit_token()
            || before.phase != (PurchasePhase::Funded {})
            || before.terms != terms.terms()?
            || output.operation() != before.operation
        {
            return Err(PublicationError::Mismatch);
        }
        match output.outcome() {
            PurchaseOutcome::Granted => {
                if self.action.key().id.as_str() != "settle_original_mind"
                    || before.participants != self.registration()?.participants
                    || u32::from(before.format) != self.action.version()
                    || !matches!(self.action.version(), 1 | 2)
                    || (self.action.version() == 1
                        && terms.form != (super::paid::PaidForm::Sealed {}))
                {
                    return Err(PublicationError::Mismatch);
                }
                let facts = facts.ok_or(PublicationError::Unavailable)?;
                self.current(ctx, txn, facts)?;
                if facts.requester() != before.holder
                    || facts.charter() != before.charter
                    || facts.line() != before.line
                    || facts.filed() != before.filed
                    || facts.revision() != before.revision
                    || facts.rating() != before.rating
                    || terms.class < facts.minimum()
                {
                    return Err(PublicationError::Mismatch);
                }
            }
            PurchaseOutcome::Refunded => {
                if self.action.key().id.as_str() != "refund_original_mind" || facts.is_some() {
                    return Err(PublicationError::Mismatch);
                }
            }
        }
        let mut after = before.clone();
        after.updated_by = output.commit();
        after.phase = match output.outcome() {
            PurchaseOutcome::Granted => PurchasePhase::Granted {
                commit: output.commit(),
            },
            PurchaseOutcome::Refunded => PurchasePhase::Refunded {
                commit: output.commit(),
            },
        };
        let roots = PaidRoots::terminal(before, &after, output, facts)?;
        let families = &self.registration()?.families;
        let licence_key = before.licence.key();
        let term_key = format!("{}/{}", licence_key, before.term.key());
        if txn
            .get(&families.paid_licences, &licence_key)
            .map_err(invalid)?
            .is_some()
            || txn
                .get(&families.paid_terms, &term_key)
                .map_err(invalid)?
                .is_some()
        {
            return Err(PublicationError::Mismatch);
        }
        if output.outcome() == PurchaseOutcome::Granted {
            let licence = PersonalPaidLicence::original(
                before.licence,
                before.charter,
                before.line,
                before.holder,
                before.term,
                before.operation,
            )?;
            let term = PaidTermRecord::original(
                before.licence,
                before.term,
                terms.window(output.issued_at_ic().ok_or(PublicationError::Mismatch)?)?,
                before.terms,
                RevisionPolicy::Pin {
                    filed: before.filed,
                    revision: before.revision,
                },
                before.operation,
            )?;
            term.matches_current(&licence)?;
            txn.insert(&families.paid_licences, &licence_key, licence.clone())
                .map_err(invalid)?;
            txn.insert(&families.paid_terms, &term_key, term.clone())
                .map_err(invalid)?;
            if txn
                .get(&families.paid_licences, &licence_key)
                .map_err(invalid)?
                != Some(&licence)
                || txn.get(&families.paid_terms, &term_key).map_err(invalid)? != Some(&term)
            {
                return Err(PublicationError::Mismatch);
            }
        }
        txn.replace(&families.paid_allocations, &before.key(), after.clone())
            .map_err(invalid)?;
        if txn
            .get(&families.paid_allocations, &before.key())
            .map_err(invalid)?
            != Some(&after)
        {
            return Err(PublicationError::Mismatch);
        }
        ctx.prepare_owner(&self.roots, RootRequest::Paid(Box::new(roots)))
            .map_err(invalid)?;
        Ok(after)
    }
}

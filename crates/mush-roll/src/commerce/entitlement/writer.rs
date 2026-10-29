//! Closed Roll half of new entitlement publication. This shares one private
//! registration cell with Market; it exposes neither handles nor arbitrary
//! row/image constructors. Every operation rereads the actual owner facts.

use std::sync::{Arc, OnceLock};

use mush_core::PrepareCtx;
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::participant::ParticipantHandle;
use mush_core::read_service::OwnerReadHandle;
use mush_core::{ActionTxn, ClockStamp, CommitToken, DefinitionRef, ModError, ObjId, Registrar};
use mush_economy::managed_entitlement::{
    EntitlementFundingOutput, EntitlementProtocol, EntitlementPurchase, EntitlementTerminalOutput,
};
use mush_economy::managed_purchase::PurchaseOutcome;

use super::super::families::CommerceFamilies;
use super::super::paid::{PaidForm, PaidTermRecord, PaidTerms, PersonalPaidLicence};
use super::super::participant::{RootParticipant, RootRequest};
use super::super::purchase::{PurchaseParticipants, PurchasePhase};
use super::super::records::{PublicationError, Record, RevisionPolicy};
use super::super::{
    LicenceId, MAX_GRACE_SECONDS, MAX_TERM_SECONDS, TermGeneration, TermWindow, WorldIcSeconds,
};
use super::reader::{EntitlementSaleFacts, EntitlementSaleReader, EntitlementSaleRequest};
use super::roots::{EntitlementRoots, Transition, action_id};
use super::{EntitlementAllocationV1, GrantEconomicsV1, PaidTermEconomicsV1};

fn invalid(_: impl std::fmt::Display) -> PublicationError {
    PublicationError::Mismatch
}
fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}

/// Explicit accepted product data. Its structural validation grants nothing;
/// Market's complete quote, E4 output and core six-target verifier also agree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntitlementPurchaseTerms {
    pub terms: PaidTerms,
    pub term_seconds: i64,
    pub grace_seconds: i64,
    pub grant: GrantEconomicsV1,
}
impl EntitlementPurchaseTerms {
    /// Structural one-seat consent only; this carries no grant or current rights.
    pub fn personal_terms(
        class: super::super::rating::RatingClass,
        form: PaidForm,
        transferable: bool,
    ) -> Result<PaidTerms, PublicationError> {
        PaidTerms::original_personal_form(class, form, transferable)
    }

    fn validate(&self) -> Result<(), PublicationError> {
        self.grant.validate()?;
        if self.terms.seats().get() != 1
            || self.grant.renewal().terms() != self.terms
            || !(1..=MAX_TERM_SECONDS).contains(&self.term_seconds)
            || !(0..=MAX_GRACE_SECONDS).contains(&self.grace_seconds)
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
    fn matches_facts(&self, facts: &EntitlementSaleFacts) -> Result<(), PublicationError> {
        self.validate()?;
        if self.terms.class() < facts.minimum()
            || self.grant.royalty_chain() != facts.royalty().reference()?
            || self.grant.renewal().revision()
                != (RevisionPolicy::Pin {
                    filed: facts.filed(),
                    revision: facts.revision(),
                })
        {
            return Err(PublicationError::Mismatch);
        }
        if self.terms.form() == (PaidForm::OpenResale {}) {
            facts.royalty().check_next_resale(
                self.grant
                    .downstream()
                    .royalty_permille()
                    .ok_or(PublicationError::Mismatch)?,
            )?;
        }
        Ok(())
    }
    fn window(&self, stamp: &ClockStamp) -> Result<TermWindow, PublicationError> {
        self.validate()?;
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

/// Fixed declaration helper, not a registration grant. Listing is distinct
/// from funding and never reserves future paid output rows.
pub fn listing_descriptor(
    protocol: EntitlementProtocol,
    manifest: &DefinitionRef,
    participants: &PurchaseParticipants,
) -> Result<DefinitionDescriptor, ModError> {
    descriptor(protocol, Transition::List, manifest, participants)
}
pub fn purchase_descriptors(
    protocol: EntitlementProtocol,
    manifest: &DefinitionRef,
    participants: &PurchaseParticipants,
) -> Result<[DefinitionDescriptor; 3], ModError> {
    Ok([
        descriptor(protocol, Transition::Fund, manifest, participants)?,
        descriptor(protocol, Transition::Grant, manifest, participants)?,
        descriptor(protocol, Transition::Refund, manifest, participants)?,
    ])
}
fn descriptor(
    protocol: EntitlementProtocol,
    transition: Transition,
    manifest: &DefinitionRef,
    participants: &PurchaseParticipants,
) -> Result<DefinitionDescriptor, ModError> {
    super::super::holding_reader::descriptor(manifest)?;
    participants.validate().map_err(error)?;
    let transition_name = match transition {
        Transition::List => "list",
        Transition::Fund => "fund",
        Transition::Grant => "grant",
        Transition::Refund => "refund",
    };
    Ok(DefinitionDescriptor::new(DefinitionKind::new("action").map_err(error)?,DefinitionId::new(action_id(protocol,transition)).map_err(error)?,protocol.version(),
        serde_json::json!({"format":protocol.version(),"protocol":protocol.name(),"transition":transition_name})).map_err(error)?
        .with_references(vec![manifest.clone(),participants.economy().clone(),participants.retention().clone()]))
}

#[derive(Clone)]
struct Registration {
    families: CommerceFamilies,
    manifest: DefinitionRef,
    participants: PurchaseParticipants,
    // Exact ordered [list, fund, grant, refund] per protocol, fixed at freeze.
    original: [DefinitionRef; 4],
    derived: [DefinitionRef; 4],
}
impl Registration {
    fn action(&self, protocol: EntitlementProtocol, transition: Transition) -> &DefinitionRef {
        let refs = match protocol {
            EntitlementProtocol::OriginalV3 => &self.original,
            EntitlementProtocol::DerivedV1 => &self.derived,
        };
        let [list, fund, grant, refund] = refs;
        match transition {
            Transition::List => list,
            Transition::Fund => fund,
            Transition::Grant => grant,
            Transition::Refund => refund,
        }
    }
}

#[derive(Clone, Default)]
pub struct EntitlementPurchaseWriter(Arc<OnceLock<Registration>>);
impl std::fmt::Debug for EntitlementPurchaseWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EntitlementPurchaseWriter")
            .field("registered", &self.0.get().is_some())
            .finish()
    }
}
impl EntitlementPurchaseWriter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Historical Market join over actual private Roll rows. Current seller or
    /// parent eligibility is deliberately irrelevant to an acknowledged birth.
    pub fn audit_allocation_for_boot(
        &self,
        ctx: &mush_core::ModuleCtx<'_>,
        operation: CommitToken,
        terms: &EntitlementPurchaseTerms,
        issued: Option<&ClockStamp>,
    ) -> Result<EntitlementAllocationV1, PublicationError> {
        let (r, roll) = self.boot_context(ctx)?;
        let row = super::super::audit::read(
            ctx,
            &r.families.entitlement_allocations,
            operation.to_hex(),
        )?;
        terms.validate()?;
        if row.operation != operation
            || row.terms != terms.terms
            || row.economics.grant() != &terms.grant
            || matches!(row.phase, PurchasePhase::Granted { .. }) != issued.is_some()
        {
            return Err(PublicationError::Mismatch);
        }
        for root in super::audit::required(row)? {
            roll.check_required_root(&root).map_err(invalid)?;
        }
        let licence_key = row.licence.key();
        let term_key = format!("{}/{}", licence_key, row.term.key());
        let licence = ctx
            .action_state(&r.families.paid_licences, &licence_key)
            .map_err(invalid)?;
        let term = ctx
            .action_state(&r.families.paid_terms, &term_key)
            .map_err(invalid)?;
        let economics = ctx
            .action_state(&r.families.term_economics, &term_key)
            .map_err(invalid)?;
        if let Some(stamp) = issued {
            let current_licence = licence.ok_or(PublicationError::Missing)?;
            // The birth remains immutable when a later exact renewal advances
            // the current pointer. Full Roll history has already opened boot;
            // recheck that pointer against the actual E4-backed renewal here.
            let expected_licence = PersonalPaidLicence::original(
                row.licence,
                row.charter,
                row.line,
                row.holder,
                current_licence.term(),
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
            let birth_pointer = PersonalPaidLicence::original(
                current_licence.id(),
                current_licence.charter(),
                current_licence.line(),
                row.holder,
                current_licence.term(),
                current_licence.operation(),
            )?;
            super::super::paid_resale::history::current_from_birth(
                ctx,
                &r.families,
                current_licence,
                super::super::entitlement::history::Birth::Entitlement(row),
            )?;
            if birth_pointer != expected_licence
                || term != Some(&expected_term)
                || economics != Some(&row.economics)
            {
                return Err(PublicationError::Mismatch);
            }
            if current_licence.term() != row.term {
                super::super::renewal::audit::verify_current_pointer(
                    ctx,
                    roll,
                    &r.families,
                    current_licence,
                )?;
            }
        } else if licence.is_some() || term.is_some() || economics.is_some() {
            return Err(PublicationError::Mismatch);
        }
        Ok(row.clone())
    }

    /// Exact immutable chain used by a retained listing or consumed quote.
    /// This cannot construct a chain, certify current rights, or return source.
    pub fn audit_royalty_for_boot(
        &self,
        ctx: &mush_core::ModuleCtx<'_>,
        expected: &super::FiledRoyaltyTermsV1,
    ) -> Result<(), PublicationError> {
        let (r, _) = self.boot_context(ctx)?;
        expected.validate()?;
        if super::super::audit::read(ctx, &r.families.filed_royalties, expected.key())? != expected
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }

    /// Mandatory reverse publication inventory, including the zero-row case.
    /// The Market boot audit must join every immutable chain to its permanent
    /// actual listing; one legacy-parent empty chain may share that listing's
    /// publication with its first derived chain.
    pub fn royalty_publications_for_boot(
        &self,
        ctx: &mush_core::ModuleCtx<'_>,
    ) -> Result<Vec<super::FiledRoyaltyTermsV1>, PublicationError> {
        let (r, _) = self.boot_context(ctx)?;
        let mut rows = Vec::new();
        super::super::audit::scan(ctx, &r.families.filed_royalties, |row| {
            rows.push(row.clone());
            Ok(())
        })?;
        Ok(rows)
    }

    fn boot_context<'a>(
        &'a self,
        ctx: &'a mush_core::ModuleCtx<'_>,
    ) -> Result<(&'a Registration, &'a crate::RollStore), PublicationError> {
        if ctx.module.as_deref() != Some("market") {
            return Err(PublicationError::Unavailable);
        }
        let roll = ctx
            .state::<crate::RollStore>()
            .ok_or(PublicationError::Unavailable)?;
        roll.commerce_ready().map_err(invalid)?;
        let registration = self.0.get().ok_or(PublicationError::Unavailable)?;
        Ok((registration, roll))
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn install(
        &self,
        reg: &mut Registrar<'_>,
        families: CommerceFamilies,
        manifest: DefinitionRef,
        participants: PurchaseParticipants,
        original: [DefinitionRef; 4],
        derived: [DefinitionRef; 4],
    ) -> Result<(), ModError> {
        if reg.module() != crate::NS || self.0.get().is_some() {
            return Err(ModError::msg(
                "entitlement writer must be installed once by Roll",
            ));
        }
        validate_refs(&manifest, &participants, &original, &derived)?;
        reg.delegate_state_family(&families.deployment, "market")?;
        reg.delegate_state_family(&families.filed_royalties, "market")?;
        reg.delegate_state_family(&families.entitlement_allocations, "market")?;
        reg.delegate_state_family(&families.paid_licences, "market")?;
        // Doc 17h cut M8d D4: the revoked principals, read by the bars in the market's steps.
        reg.delegate_state_family(&families.revoked_principals, "market")?;
        // 17g C2f: the grant puts the holder's holdings row beside the pointer.
        reg.delegate_state_family(&families.paid_holdings, "market")?;
        reg.delegate_state_family(&families.paid_terms, "market")?;
        reg.delegate_state_family(&families.term_economics, "market")?;
        self.0
            .set(Registration {
                families,
                manifest,
                participants,
                original,
                derived,
            })
            .map_err(|_| ModError::msg("entitlement writer registration conflicts"))
    }
    pub fn validate_registration(
        &self,
        manifest: &DefinitionRef,
        participants: &PurchaseParticipants,
        original: &[DefinitionRef; 4],
        derived: &[DefinitionRef; 4],
    ) -> Result<(), ModError> {
        validate_refs(manifest, participants, original, derived)?;
        let actual = self
            .0
            .get()
            .ok_or_else(|| ModError::msg("Roll entitlement writer is not registered"))?;
        if manifest != &actual.manifest
            || participants != &actual.participants
            || original != &actual.original
            || derived != &actual.derived
        {
            return Err(ModError::msg("entitlement writer composition differs"));
        }
        Ok(())
    }
    pub fn for_action(
        &self,
        reg: &mut Registrar<'_>,
        action: &DefinitionRef,
        manifest: &DefinitionRef,
        participants: &PurchaseParticipants,
        reader: OwnerReadHandle<EntitlementSaleReader>,
    ) -> Result<RegisteredEntitlementWriter, ModError> {
        if reg.module() != "market" {
            return Err(ModError::msg(
                "entitlement writer requires Market registration",
            ));
        }
        let writer = RegisteredEntitlementWriter {
            action: action.clone(),
            manifest: manifest.clone(),
            participants: participants.clone(),
            registration: self.clone(),
            roots: reg.request_owner_participant::<RootParticipant>(action.clone())?,
            reader,
        };
        if self.0.get().is_some() {
            writer.registration().map_err(error)?;
        }
        Ok(writer)
    }
}
pub(crate) fn validate_refs(
    manifest: &DefinitionRef,
    participants: &PurchaseParticipants,
    original: &[DefinitionRef; 4],
    derived: &[DefinitionRef; 4],
) -> Result<(), ModError> {
    for (protocol, refs) in [
        (EntitlementProtocol::OriginalV3, original),
        (EntitlementProtocol::DerivedV1, derived),
    ] {
        for (transition, actual) in [
            Transition::List,
            Transition::Fund,
            Transition::Grant,
            Transition::Refund,
        ]
        .into_iter()
        .zip(refs)
        {
            let expected = DefinitionRegistryBuilder::new()
                .register(
                    OwnerId::new("market").map_err(error)?,
                    descriptor(protocol, transition, manifest, participants)?,
                )
                .map_err(error)?;
            if actual != &expected {
                return Err(ModError::msg(
                    "entitlement action differs from closed protocol",
                ));
            }
        }
    }
    Ok(())
}

/// Non-serializable same-preparation read evidence. The data accessor grants
/// no new read/write authority; publication validates its exact action/commit.
pub struct PreparedEntitlementSale {
    action: DefinitionRef,
    commit: CommitToken,
    facts: EntitlementSaleFacts,
}
impl PreparedEntitlementSale {
    pub fn facts(&self) -> &EntitlementSaleFacts {
        &self.facts
    }
}

pub struct RegisteredEntitlementWriter {
    action: DefinitionRef,
    manifest: DefinitionRef,
    participants: PurchaseParticipants,
    registration: EntitlementPurchaseWriter,
    roots: ParticipantHandle<RootParticipant>,
    reader: OwnerReadHandle<EntitlementSaleReader>,
}
impl RegisteredEntitlementWriter {
    fn registration(&self) -> Result<&Registration, PublicationError> {
        let r = self
            .registration
            .0
            .get()
            .ok_or(PublicationError::Unavailable)?;
        if self.manifest != r.manifest
            || self.participants != r.participants
            || !r
                .original
                .iter()
                .chain(&r.derived)
                .any(|a| a == &self.action)
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(r)
    }
    fn require_action(
        &self,
        protocol: EntitlementProtocol,
        transition: Transition,
    ) -> Result<(), PublicationError> {
        if self.registration()?.action(protocol, transition) != &self.action {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
    /// One actual preimage read, retained by the same ActionTxn. The opaque
    /// observation cannot be supplied from wire data or another preparation.
    pub fn observe_listing(
        &self,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        licence: LicenceId,
    ) -> Result<PreparedEntitlementSale, PublicationError> {
        let facts = ctx
            .read_owner_preimage(
                txn,
                &self.reader,
                EntitlementSaleRequest::own(licence, ctx.commit_token()),
            )
            .map_err(invalid)?;
        self.require_action(facts.protocol(), Transition::List)?;
        if facts.requester() != facts.seller() {
            return Err(PublicationError::Mismatch);
        }
        Ok(PreparedEntitlementSale {
            action: self.action.clone(),
            commit: ctx.commit_token(),
            facts,
        })
    }
    pub fn observe_purchase(
        &self,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        seller: ObjId,
        licence: LicenceId,
    ) -> Result<PreparedEntitlementSale, PublicationError> {
        let facts = ctx
            .read_owner_preimage(
                txn,
                &self.reader,
                EntitlementSaleRequest::listed(seller, licence),
            )
            .map_err(invalid)?;
        let r = self.registration()?;
        if (&self.action != r.action(facts.protocol(), Transition::Fund)
            && &self.action != r.action(facts.protocol(), Transition::Grant))
            || !facts.materialization().creates().is_empty()
        {
            return Err(PublicationError::Mismatch);
        }
        let pair = ctx
            .distinct_personal_accounts(facts.seller())
            .map_err(invalid)?;
        if pair.requester() != facts.requester() || pair.other() != facts.seller() {
            return Err(PublicationError::Mismatch);
        }
        Ok(PreparedEntitlementSale {
            action: self.action.clone(),
            commit: ctx.commit_token(),
            facts,
        })
    }
    fn observed<'a>(
        &self,
        ctx: &PrepareCtx<'_>,
        observation: &'a PreparedEntitlementSale,
    ) -> Result<&'a EntitlementSaleFacts, PublicationError> {
        self.registration()?;
        if observation.action != self.action || observation.commit != ctx.commit_token() {
            return Err(PublicationError::Mismatch);
        }
        Ok(&observation.facts)
    }
    pub fn materialize_listing(
        &self,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        observation: &PreparedEntitlementSale,
    ) -> Result<(), PublicationError> {
        let facts = self.observed(ctx, observation)?;
        self.require_action(facts.protocol(), Transition::List)?;
        if facts.seller() != facts.requester() {
            return Err(PublicationError::Mismatch);
        }
        let f = &self.registration()?.families;
        for row in facts.materialization().creates() {
            if row.publication() != ctx.commit_token()
                || txn
                    .get(&f.filed_royalties, &row.key())
                    .map_err(invalid)?
                    .is_some()
            {
                return Err(PublicationError::Mismatch);
            }
            txn.insert(&f.filed_royalties, &row.key(), row.clone())
                .map_err(invalid)?;
            if txn.get(&f.filed_royalties, &row.key()).map_err(invalid)? != Some(row) {
                return Err(PublicationError::Mismatch);
            }
        }
        if txn
            .get(&f.filed_royalties, &facts.royalty().key())
            .map_err(invalid)?
            != Some(facts.royalty())
        {
            return Err(PublicationError::Mismatch);
        }
        let roots = EntitlementRoots::listing(&self.action, ctx.commit_token(), facts)?;
        ctx.prepare_owner(&self.roots, RootRequest::Entitlement(Box::new(roots)))
            .map_err(invalid)?;
        Ok(())
    }
    pub fn allocation(
        &self,
        txn: &mut ActionTxn<'_>,
        operation: CommitToken,
    ) -> Result<EntitlementAllocationV1, PublicationError> {
        let f = &self.registration()?.families;
        let row = txn
            .get(&f.entitlement_allocations, &operation.to_hex())
            .map_err(invalid)?
            .cloned()
            .ok_or(PublicationError::Missing)?;
        row.validate()?;
        if row.operation != operation {
            return Err(PublicationError::Mismatch);
        }
        Ok(row)
    }
    /// Before any E4 funding preparation, prove that the admitted economics
    /// can fit its complete typed envelope at every later term generation.
    /// This only reads the real allocator; it creates neither a grant nor a
    /// renewal permission. Historical decoders remain byte-for-byte unchanged.
    pub fn preflight_funding(
        &self,
        ctx: &PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        purchase: &EntitlementPurchase,
        terms: &EntitlementPurchaseTerms,
        observation: &PreparedEntitlementSale,
    ) -> Result<(), PublicationError> {
        self.require_action(purchase.protocol, Transition::Fund)?;
        let facts = self.observed(ctx, observation)?;
        terms.matches_facts(facts)?;
        check_purchase(purchase, terms, facts)?;
        let f = &self.registration()?.families;
        let deployment = txn
            .get(&f.deployment, &"registry".into())
            .map_err(invalid)?
            .ok_or(PublicationError::Unavailable)?;
        deployment.validate()?;
        let licence = LicenceId::new(deployment.next_licence).map_err(invalid)?;
        let row = PaidTermEconomicsV1::capture(
            licence,
            TermGeneration::new(1).map_err(invalid)?,
            ctx.commit_token(),
            purchase.product_digest,
            terms.grant.clone(),
        )?;
        super::admission_capacity::check(&f.term_economics, &row)
    }
    pub fn fund(
        &self,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        purchase: &EntitlementPurchase,
        terms: &EntitlementPurchaseTerms,
        observation: &PreparedEntitlementSale,
        output: &EntitlementFundingOutput,
    ) -> Result<EntitlementAllocationV1, PublicationError> {
        self.require_action(purchase.protocol, Transition::Fund)?;
        let facts = self.observed(ctx, observation)?;
        terms.matches_facts(facts)?;
        check_purchase(purchase, terms, facts)?;
        // Doc 17h cut M8d fix round K8: a revoked buyer is refused before money is held.
        super::super::registry_inspection::principal::refuse(
            ctx,
            txn,
            &self.registration()?.families,
            purchase.buyer,
        )?;
        let commit = ctx.commit_token();
        let marker = output.marker();
        if output.operation() != commit
            || marker.effect() != &self.action
            || marker.operation() != commit
            || marker.quote() != purchase.quote
            || marker.quote_digest() != purchase.quote_digest
            || marker.product_digest() != purchase.product_digest
            || marker.entitlement_protocol() != Some(purchase.protocol)
            || marker.economic_digest() != Some(purchase.economic_digest().map_err(invalid)?)
        {
            return Err(PublicationError::Mismatch);
        }
        let f = &self.registration()?.families;
        let mut deployment = txn
            .get(&f.deployment, &"registry".into())
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
        let row = EntitlementAllocationV1 {
            format: 1,
            protocol: purchase.protocol,
            operation: commit,
            participants: self.participants.clone(),
            licence,
            term,
            quote: purchase.quote,
            quote_digest: purchase.quote_digest,
            product_digest: purchase.product_digest,
            // M8c B3: the holder is the quote's buyer, a character or an
            // organisation acting through this invocation's actor.
            holder: purchase.buyer,
            charter: facts.charter(),
            line: facts.line(),
            filed: facts.filed(),
            revision: facts.revision(),
            rating: facts.rating(),
            terms: terms.terms,
            economics: PaidTermEconomicsV1::capture(
                licence,
                term,
                commit,
                purchase.product_digest,
                terms.grant.clone(),
            )?,
            funding: output.receipt().clone(),
            phase: PurchasePhase::Funded {},
            updated_by: commit,
        };
        row.validate()?;
        super::admission_capacity::check(&f.term_economics, &row.economics)?;
        txn.replace(&f.deployment, &"registry".into(), deployment)
            .map_err(invalid)?;
        txn.insert(&f.entitlement_allocations, &row.key(), row.clone())
            .map_err(invalid)?;
        if txn
            .get(&f.entitlement_allocations, &row.key())
            .map_err(invalid)?
            != Some(&row)
        {
            return Err(PublicationError::Mismatch);
        }
        ctx.prepare_owner(
            &self.roots,
            RootRequest::Entitlement(Box::new(EntitlementRoots::funding(
                &self.action,
                &row,
                commit,
                facts,
            )?)),
        )
        .map_err(invalid)?;
        Ok(row)
    }
    // Requester is data from the provider invocation; the private participant
    // checks it against the actual core-stamped invocation before custody.
    #[allow(clippy::too_many_arguments)]
    pub fn terminal(
        &self,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        requester: ObjId,
        before: &EntitlementAllocationV1,
        terms: &EntitlementPurchaseTerms,
        observation: Option<&PreparedEntitlementSale>,
        output: &EntitlementTerminalOutput,
    ) -> Result<EntitlementAllocationV1, PublicationError> {
        terms.validate()?;
        let facts = observation
            .map(|value| self.observed(ctx, value))
            .transpose()?;
        if self.allocation(txn, before.operation)? != *before
            || before.phase != (PurchasePhase::Funded {})
            || before.terms != terms.terms
            || before.economics.grant() != &terms.grant
            || output.commit() != ctx.commit_token()
            || output.operation() != before.operation
        {
            return Err(PublicationError::Mismatch);
        }
        match output.outcome() {
            PurchaseOutcome::Granted => {
                self.require_action(before.protocol, Transition::Grant)?;
                if before.participants != self.participants {
                    return Err(PublicationError::Mismatch);
                }
                let facts = facts.ok_or(PublicationError::Unavailable)?;
                terms.matches_facts(facts)?;
            }
            PurchaseOutcome::Refunded => {
                self.require_action(before.protocol, Transition::Refund)?;
                if facts.is_some() {
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
        let roots =
            EntitlementRoots::terminal(&self.action, requester, before, &after, output, facts)?;
        let f = &self.registration()?.families;
        let licence_key = before.licence.key();
        let term_key = format!("{}/{}", licence_key, before.term.key());
        if txn
            .get(&f.paid_licences, &licence_key)
            .map_err(invalid)?
            .is_some()
            || txn
                .get(&f.paid_terms, &term_key)
                .map_err(invalid)?
                .is_some()
            || txn
                .get(&f.term_economics, &term_key)
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
            // Doc 17h cut M8d D4: a principal the Registry revoked holds no new licence.
            super::super::registry_inspection::principal::refuse(ctx, txn, f, licence.holder())?;
            txn.insert(&f.paid_licences, &licence_key, licence.clone())
                .map_err(invalid)?;
            txn.insert(&f.paid_terms, &term_key, term.clone())
                .map_err(invalid)?;
            txn.insert(&f.term_economics, &term_key, before.economics.clone())
                .map_err(invalid)?;
            if txn.get(&f.paid_licences, &licence_key).map_err(invalid)? != Some(&licence)
                || txn.get(&f.paid_terms, &term_key).map_err(invalid)? != Some(&term)
                || txn.get(&f.term_economics, &term_key).map_err(invalid)?
                    != Some(&before.economics)
            {
                return Err(PublicationError::Mismatch);
            }
            // 17g C2f (the C2b brief §2): the holder's paid-holdings row,
            // in the grant's own transaction (mush-core admits exactly it).
            super::super::paid_holdings::grant(txn, f, before.holder, before.licence)?;
        }
        txn.replace(&f.entitlement_allocations, &before.key(), after.clone())
            .map_err(invalid)?;
        if txn
            .get(&f.entitlement_allocations, &before.key())
            .map_err(invalid)?
            != Some(&after)
        {
            return Err(PublicationError::Mismatch);
        }
        ctx.prepare_owner(&self.roots, RootRequest::Entitlement(Box::new(roots)))
            .map_err(invalid)?;
        Ok(after)
    }
}

fn check_purchase(
    p: &EntitlementPurchase,
    terms: &EntitlementPurchaseTerms,
    facts: &EntitlementSaleFacts,
) -> Result<(), PublicationError> {
    if p.protocol != facts.protocol()
        || p.buyer.acting_character() != facts.requester()
        || p.seller != facts.seller()
        || p.charter != mush_core::ObjId(facts.charter().get())
        || p.title_revision != facts.title_revision()
        || p.royalty_chain_digest != facts.royalty().digest()?
        || p.grant_economics_digest != terms.grant.digest()?
        || p.parents.len() != facts.royalty().edges().len()
        || p.parents
            .iter()
            .zip(facts.royalty().edges())
            .any(|(cut, edge)| {
                cut.charter != mush_core::ObjId(edge.charter().get())
                    || cut.permille != edge.royalty_permille()
            })
    {
        return Err(PublicationError::Mismatch);
    }
    Ok(())
}

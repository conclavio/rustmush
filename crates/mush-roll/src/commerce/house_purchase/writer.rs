//! Closed Roll half of new entitlement publication. This shares one private
//! registration cell with Market; it exposes neither handles nor arbitrary
//! row/image constructors. Every operation rereads the actual owner facts.

use std::sync::{Arc, OnceLock};

use super::HouseProtocol;
use mush_core::PrepareCtx;
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::participant::ParticipantHandle;
use mush_core::read_service::OwnerReadHandle;
use mush_core::{ActionTxn, ClockStamp, CommitToken, DefinitionRef, ModError, ObjId, Registrar};
use mush_economy::managed_house_purchase::{
    HouseFundingOutput, HousePurchase, HouseTerminalOutput,
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
use super::roots::{HouseRoots, Transition, action_id};
use super::{HouseAllocationV1, HouseGrantEconomicsV1, HouseTermEconomicsV1};
use crate::commerce::house_catalogue::{
    HouseIssuer, HouseSaleFacts, HouseSaleReader, HouseSaleRequest,
};

fn invalid(_: impl std::fmt::Display) -> PublicationError {
    PublicationError::Mismatch
}
fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}

/// Explicit accepted product data. Its structural validation grants nothing;
/// Market's complete quote, E4 output and core six-target verifier also agree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HousePurchaseTerms {
    pub terms: PaidTerms,
    pub term_seconds: i64,
    pub grace_seconds: i64,
    pub grant: HouseGrantEconomicsV1,
}
impl HousePurchaseTerms {
    /// Structural one-seat consent only; this carries no grant or current rights.
    pub fn personal_terms(
        class: super::super::rating::RatingClass,
        transferable: bool,
    ) -> Result<PaidTerms, PublicationError> {
        PaidTerms::original_personal_form(class, PaidForm::Sealed {}, transferable)
    }

    pub fn personal_terms_for_protocol(
        protocol: HouseProtocol,
        class: super::super::rating::RatingClass,
        transferable: bool,
    ) -> Result<PaidTerms, PublicationError> {
        match protocol {
            HouseProtocol::HouseV1 => Self::personal_terms(class, transferable),
            HouseProtocol::HouseV2 => PaidTerms::house_two_seat_pin(class, transferable),
        }
    }
    fn protocol(&self) -> Result<HouseProtocol, PublicationError> {
        HouseProtocol::from_format(self.grant.format())
    }
    fn validate(&self) -> Result<(), PublicationError> {
        self.grant.validate()?;
        if self.terms.seats().get() != self.protocol()?.seats()
            || self.terms.form() != (PaidForm::Sealed {})
            || self.grant.renewal().terms() != self.terms
            || !(1..=MAX_TERM_SECONDS).contains(&self.term_seconds)
            || !(0..=MAX_GRACE_SECONDS).contains(&self.grace_seconds)
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
    fn matches_facts(&self, facts: &HouseSaleFacts) -> Result<(), PublicationError> {
        self.validate()?;
        if self.terms.class() < facts.minimum()
            || self.grant.renewal().revision()
                != (RevisionPolicy::Pin {
                    filed: facts.filed(),
                    revision: facts.revision(),
                })
        {
            return Err(PublicationError::Mismatch);
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
    protocol: HouseProtocol,
    manifest: &DefinitionRef,
    participants: &PurchaseParticipants,
) -> Result<DefinitionDescriptor, ModError> {
    descriptor(protocol, Transition::List, manifest, participants)
}
pub fn purchase_descriptors(
    protocol: HouseProtocol,
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
    protocol: HouseProtocol,
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
        serde_json::json!({"format":protocol.format(),"protocol":protocol.protocol_name(),"transition":transition_name})).map_err(error)?
        .with_references(vec![manifest.clone(),participants.economy().clone(),participants.retention().clone()]))
}

#[derive(Clone)]
struct Registration {
    families: CommerceFamilies,
    ratings: super::super::rating::RatingFamilies,
    manifest: DefinitionRef,
    participants: PurchaseParticipants,
    // Exact ordered [list, fund, grant, refund] per protocol, fixed at freeze.
    actions: [DefinitionRef; 4],
    two_seat_actions: Option<[DefinitionRef; 4]>,
}
impl Registration {
    fn actions(&self, protocol: HouseProtocol) -> Result<&[DefinitionRef; 4], PublicationError> {
        match protocol {
            HouseProtocol::HouseV1 => Ok(&self.actions),
            HouseProtocol::HouseV2 => self
                .two_seat_actions
                .as_ref()
                .ok_or(PublicationError::Unavailable),
        }
    }
    fn action(
        &self,
        protocol: HouseProtocol,
        transition: Transition,
    ) -> Result<&DefinitionRef, PublicationError> {
        let refs = self.actions(protocol)?;
        let [list, fund, grant, refund] = refs;
        Ok(match transition {
            Transition::List => list,
            Transition::Fund => fund,
            Transition::Grant => grant,
            Transition::Refund => refund,
        })
    }
}

#[derive(Clone, Default)]
pub struct HousePurchaseWriter(
    Arc<OnceLock<Registration>>,
    Option<Arc<dyn super::HouseBuyerAdmission>>,
);
impl std::fmt::Debug for HousePurchaseWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HousePurchaseWriter")
            .field("registered", &self.0.get().is_some())
            .finish()
    }
}
/// Source-free historical comparison data. Constructing it grants no access.
#[derive(Clone, Debug)]
pub struct HouseListingIdentity {
    pub issuer: HouseIssuer,
    pub charter: super::super::CharterId,
    pub line: super::super::LineId,
    pub filed: super::super::FiledRevisionId,
    pub sequence: u64,
    pub owner_licence: LicenceId,
    pub owner_term: TermGeneration,
    pub owner_window: TermWindow,
    pub revision: super::super::RevisionIdentity,
    pub schema: mush_core::DefinitionDigest,
    pub definition: mush_core::DefinitionDigest,
    pub rating: super::super::rating::RatingRef,
    pub compatibility: mush_core::DefinitionDigest,
    pub declared: super::super::rating::RatingClass,
    pub minimum: super::super::rating::RatingClass,
    pub title_revision: u64,
}
impl HousePurchaseWriter {
    /// Repeat historical identity joins without querying current publisher,
    /// organization standing, licence phase or the current certificate head.
    pub fn audit_listing_for_boot(
        &self,
        ctx: &mush_core::ModuleCtx<'_>,
        subject: &HouseListingIdentity,
    ) -> Result<(), PublicationError> {
        use super::super::audit::read;
        use super::super::roots::{RequiredRoot, RootAnchor};
        let (r, roll) = self.boot_context(ctx)?;
        let charter = read(ctx, &r.families.charters, subject.charter.key())?;
        let birth = read(ctx, &r.families.operations, charter.operation.to_hex())?;
        let house = birth
            .economic
            .house
            .as_ref()
            .ok_or(PublicationError::Mismatch)?;
        if birth.format != 2
            || birth.parent.is_some()
            || birth.charter != subject.charter
            || birth.line != subject.line
            || birth.licence != subject.owner_licence
            || birth.economic.issued_owner != subject.issuer.principal()
            || house.organization() != subject.issuer.organization()
            || house.scope() != subject.issuer.scope()
            || birth.economic.title_revision != subject.title_revision
        {
            return Err(PublicationError::Mismatch);
        }
        ctx.state::<mush_economy::EconomyStore>()
            .ok_or(PublicationError::Unavailable)?
            .filing_receipt_audit()
            .verify(&birth.audit_subject()?)
            .map_err(invalid)?;
        let term = super::super::owner_attestation::history::historical(
            ctx,
            roll,
            &r.families,
            birth,
            subject.owner_term,
        )?;
        let filed = read(ctx, &r.families.filed, subject.filed.key())?;
        let line = read(ctx, &r.families.lines, subject.line.key())?;
        if term.window != subject.owner_window
            || term.parent.is_some()
            || filed.content.parent().is_some()
            || filed.content.revision() != subject.revision
            || filed.content.position().charter != subject.charter
            || filed.content.position().line != subject.line
            || filed.content.position().sequence.get() != subject.sequence
            || filed.definition != subject.definition
            || line.schema_digest != subject.schema
            || super::super::line_update::audit::birth(ctx, &r.families, filed)?.commit
                != birth.commit
        {
            return Err(PublicationError::Mismatch);
        }
        match term.revision {
            RevisionPolicy::Pin { filed, revision }
                if filed == subject.filed && revision == subject.revision => {}
            RevisionPolicy::Follow { line } if line == subject.line => {}
            _ => return Err(PublicationError::Mismatch),
        }
        let rated = super::super::rating::reconformance::history::certificate(
            ctx,
            &r.ratings,
            filed,
            &subject.rating,
        )
        .map_err(invalid)?;
        if rated.subject().revision() != subject.revision
            || rated.subject().schema() != subject.schema
            || rated.subject().definition() != subject.definition
            || rated.subject().declared() != subject.declared
            || rated.minimum() != subject.minimum
            || rated.compatibility().digest() != subject.compatibility
        {
            return Err(PublicationError::Mismatch);
        }
        for anchor in [
            RootAnchor::Filed(subject.filed),
            RootAnchor::Rating {
                job: rated.publication().job(),
                id: rated.id(),
            },
        ] {
            roll.check_required_root(&RequiredRoot {
                anchor,
                identity: subject.revision,
            })
            .map_err(invalid)?;
        }
        Ok(())
    }
    pub fn new() -> Self {
        Self::default()
    }
    /// Trusted host configuration only. This check cannot add any authority and
    /// is retained by the actual Root participant for new Fund/Grant custody.
    pub(crate) fn buyer_admission(&self) -> Option<Arc<dyn super::HouseBuyerAdmission>> {
        self.1.clone()
    }
    pub fn with_buyer_admission(
        mut self,
        policy: Arc<dyn super::HouseBuyerAdmission>,
    ) -> Result<Self, ModError> {
        if self.0.get().is_some() || self.1.is_some() {
            return Err(error(
                "House buyer policy must be configured once before registration",
            ));
        }
        self.1 = Some(policy);
        Ok(self)
    }
    pub fn check_buyer(
        &self,
        world: &mush_core::World,
        issuer: HouseIssuer,
        buyer: ObjId,
    ) -> Result<(), PublicationError> {
        if let Some(policy) = &self.1 {
            policy.check(world, issuer, buyer)?;
        }
        Ok(())
    }

    /// Historical Market join over actual private Roll rows. Current seller or
    /// parent eligibility is deliberately irrelevant to an acknowledged birth.
    pub fn audit_allocation_for_boot(
        &self,
        ctx: &mush_core::ModuleCtx<'_>,
        operation: CommitToken,
        terms: &HousePurchaseTerms,
        issued: Option<&ClockStamp>,
    ) -> Result<HouseAllocationV1, PublicationError> {
        let (r, roll) = self.boot_context(ctx)?;
        let row =
            super::super::audit::read(ctx, &r.families.house_allocations, operation.to_hex())?;
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
            .action_state(&r.families.house_economics, &term_key)
            .map_err(invalid)?;
        if let Some(stamp) = issued {
            let current_licence = licence.ok_or(PublicationError::Missing)?;
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
            let birth_licence = PersonalPaidLicence::original(
                current_licence.id(),
                current_licence.charter(),
                current_licence.line(),
                row.holder,
                row.term,
                current_licence.operation(),
            )?;
            super::super::paid_resale::history::current_from_birth(
                ctx,
                &r.families,
                current_licence,
                super::super::entitlement::history::Birth::House(row),
            )?;
            if birth_licence != expected_licence
                || current_licence.term().get() < row.term.get()
                || term != Some(&expected_term)
                || economics != Some(&row.economics)
            {
                return Err(PublicationError::Mismatch);
            }
            if current_licence.term().get() > 1 {
                super::super::house_renewal::audit::verify_current_pointer(
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
        ratings: super::super::rating::RatingFamilies,
        manifest: DefinitionRef,
        participants: PurchaseParticipants,
        actions: [DefinitionRef; 4],
        two_seat_actions: Option<[DefinitionRef; 4]>,
    ) -> Result<(), ModError> {
        if reg.module() != crate::NS || self.0.get().is_some() {
            return Err(ModError::msg(
                "entitlement writer must be installed once by Roll",
            ));
        }
        validate_refs(&manifest, &participants, &actions)?;
        if let Some(refs) = &two_seat_actions {
            validate_refs_for_protocol(HouseProtocol::HouseV2, &manifest, &participants, refs)?;
        }
        reg.delegate_state_family(&families.deployment, "market")?;
        reg.delegate_state_family(&families.house_allocations, "market")?;
        reg.delegate_state_family(&families.paid_licences, "market")?;
        reg.delegate_state_family(&families.paid_terms, "market")?;
        reg.delegate_state_family(&families.house_economics, "market")?;
        // Market's boot audit repeats each listing's historical identity join
        // (`audit_listing_for_boot`) from its own ModuleCtx: read-only grants
        // for exactly the families that join reaches; Market writes none.
        reg.grant_state_family_read(&families.charters, "market")?;
        reg.grant_state_family_read(&families.operations, "market")?;
        reg.grant_state_family_read(&families.terms, "market")?;
        reg.grant_state_family_read(&families.owner_attestations, "market")?;
        reg.grant_state_family_read(&families.charter_transfers, "market")?;
        reg.grant_state_family_read(&families.charter_transfer_offers, "market")?;
        reg.grant_state_family_read(&families.charter_pending, "market")?;
        reg.grant_state_family_read(&families.charter_pending_offers, "market")?;
        reg.grant_state_family_read(&families.filed, "market")?;
        reg.grant_state_family_read(&families.lines, "market")?;
        reg.grant_state_family_read(&families.line_updates, "market")?;
        reg.grant_state_family_read(&families.update_positions, "market")?;
        reg.grant_state_family_read(&ratings.receipts, "market")?;
        reg.grant_state_family_read(&ratings.jobs, "market")?;
        reg.grant_state_family_read(&ratings.executions, "market")?;
        self.0
            .set(Registration {
                families,
                ratings,
                manifest,
                participants,
                actions,
                two_seat_actions,
            })
            .map_err(|_| ModError::msg("entitlement writer registration conflicts"))
    }
    pub fn validate_registration(
        &self,
        manifest: &DefinitionRef,
        participants: &PurchaseParticipants,
        actions: &[DefinitionRef; 4],
    ) -> Result<(), ModError> {
        self.validate_registration_with_two_seat_pins(manifest, participants, actions, None)
    }
    pub fn validate_registration_with_two_seat_pins(
        &self,
        manifest: &DefinitionRef,
        participants: &PurchaseParticipants,
        actions: &[DefinitionRef; 4],
        two_seat_actions: Option<&[DefinitionRef; 4]>,
    ) -> Result<(), ModError> {
        validate_refs(manifest, participants, actions)?;
        if let Some(refs) = two_seat_actions {
            validate_refs_for_protocol(HouseProtocol::HouseV2, manifest, participants, refs)?;
        }
        let actual = self
            .0
            .get()
            .ok_or_else(|| ModError::msg("Roll entitlement writer is not registered"))?;
        if manifest != &actual.manifest
            || participants != &actual.participants
            || actions != &actual.actions
            || two_seat_actions != actual.two_seat_actions.as_ref()
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
        reader: OwnerReadHandle<HouseSaleReader>,
    ) -> Result<RegisteredHouseWriter, ModError> {
        if reg.module() != "market" {
            return Err(ModError::msg(
                "entitlement writer requires Market registration",
            ));
        }
        let writer = RegisteredHouseWriter {
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
    actions: &[DefinitionRef; 4],
) -> Result<(), ModError> {
    validate_refs_for_protocol(HouseProtocol::HouseV1, manifest, participants, actions)
}
pub(crate) fn validate_refs_for_protocol(
    protocol: HouseProtocol,
    manifest: &DefinitionRef,
    participants: &PurchaseParticipants,
    actions: &[DefinitionRef; 4],
) -> Result<(), ModError> {
    for refs in [actions] {
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
pub struct PreparedHouseSale {
    action: DefinitionRef,
    commit: CommitToken,
    facts: HouseSaleFacts,
}
impl PreparedHouseSale {
    pub fn facts(&self) -> &HouseSaleFacts {
        &self.facts
    }
}

pub struct RegisteredHouseWriter {
    action: DefinitionRef,
    manifest: DefinitionRef,
    participants: PurchaseParticipants,
    registration: HousePurchaseWriter,
    roots: ParticipantHandle<RootParticipant>,
    reader: OwnerReadHandle<HouseSaleReader>,
}
impl RegisteredHouseWriter {
    fn registration(&self) -> Result<&Registration, PublicationError> {
        let r = self
            .registration
            .0
            .get()
            .ok_or(PublicationError::Unavailable)?;
        if self.manifest != r.manifest
            || self.participants != r.participants
            || !r
                .actions
                .iter()
                .chain(r.two_seat_actions.iter().flatten())
                .any(|a| a == &self.action)
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(r)
    }
    fn require_action(
        &self,
        protocol: HouseProtocol,
        transition: Transition,
    ) -> Result<(), PublicationError> {
        if self.registration()?.action(protocol, transition)? != &self.action {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
    fn protocol(&self) -> Result<HouseProtocol, PublicationError> {
        let r = self.registration()?;
        if r.actions.contains(&self.action) {
            return Ok(HouseProtocol::HouseV1);
        }
        if r.two_seat_actions
            .as_ref()
            .is_some_and(|a| a.contains(&self.action))
        {
            return Ok(HouseProtocol::HouseV2);
        }
        Err(PublicationError::Mismatch)
    }
    /// One actual preimage read, retained by the same ActionTxn. The opaque
    /// observation cannot be supplied from wire data or another preparation.
    pub fn observe_listing(
        &self,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        licence: LicenceId,
    ) -> Result<PreparedHouseSale, PublicationError> {
        let facts = ctx
            .read_owner_preimage(txn, &self.reader, HouseSaleRequest::publication(licence))
            .map_err(invalid)?;
        self.require_action(self.protocol()?, Transition::List)?;
        Ok(PreparedHouseSale {
            action: self.action.clone(),
            commit: ctx.commit_token(),
            facts,
        })
    }
    pub fn observe_purchase(
        &self,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        issuer: HouseIssuer,
        licence: LicenceId,
    ) -> Result<PreparedHouseSale, PublicationError> {
        let facts = ctx
            .read_owner_preimage(txn, &self.reader, HouseSaleRequest::listed(licence, issuer))
            .map_err(invalid)?;
        let r = self.registration()?;
        if (&self.action != r.action(self.protocol()?, Transition::Fund)?
            && &self.action != r.action(self.protocol()?, Transition::Grant)?)
        {
            return Err(PublicationError::Mismatch);
        }
        self.registration.check_buyer(
            ctx.world().map_err(invalid)?,
            facts.issuer(),
            facts.requester(),
        )?;
        let account = ctx.personal_account().map_err(invalid)?;
        if account.requester() != facts.requester() {
            return Err(PublicationError::Mismatch);
        }
        Ok(PreparedHouseSale {
            action: self.action.clone(),
            commit: ctx.commit_token(),
            facts,
        })
    }
    fn observed<'a>(
        &self,
        ctx: &PrepareCtx<'_>,
        observation: &'a PreparedHouseSale,
    ) -> Result<&'a HouseSaleFacts, PublicationError> {
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
        observation: &PreparedHouseSale,
    ) -> Result<(), PublicationError> {
        let facts = self.observed(ctx, observation)?;
        self.require_action(self.protocol()?, Transition::List)?;
        let roots = HouseRoots::listing(&self.action, self.protocol()?, ctx.commit_token(), facts)?;
        ctx.prepare_owner(&self.roots, RootRequest::House(Box::new(roots)))
            .map_err(invalid)?;
        Ok(())
    }
    pub fn allocation(
        &self,
        txn: &mut ActionTxn<'_>,
        operation: CommitToken,
    ) -> Result<HouseAllocationV1, PublicationError> {
        let f = &self.registration()?.families;
        let row = txn
            .get(&f.house_allocations, &operation.to_hex())
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
        purchase: &HousePurchase,
        terms: &HousePurchaseTerms,
        observation: &PreparedHouseSale,
    ) -> Result<(), PublicationError> {
        self.require_action(terms.protocol()?, Transition::Fund)?;
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
        let row = HouseTermEconomicsV1::capture(
            licence,
            TermGeneration::new(1).map_err(invalid)?,
            ctx.commit_token(),
            purchase.product_digest,
            terms.grant.clone(),
        )?;
        super::admission_capacity::check(&f.house_economics, &row)
    }
    pub fn fund(
        &self,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        purchase: &HousePurchase,
        terms: &HousePurchaseTerms,
        observation: &PreparedHouseSale,
        output: &HouseFundingOutput,
    ) -> Result<HouseAllocationV1, PublicationError> {
        self.require_action(terms.protocol()?, Transition::Fund)?;
        let facts = self.observed(ctx, observation)?;
        terms.matches_facts(facts)?;
        check_purchase(purchase, terms, facts)?;
        let commit = ctx.commit_token();
        let marker = output.marker();
        if output.operation() != commit
            || marker.effect() != &self.action
            || marker.operation() != commit
            || marker.quote() != purchase.quote
            || marker.quote_digest() != purchase.quote_digest
            || marker.product_digest() != purchase.product_digest
            || marker.house_economic_digest() != Some(purchase.economic_digest().map_err(invalid)?)
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
        let row = HouseAllocationV1 {
            format: terms.protocol()?.format(),
            protocol: terms.protocol()?,
            operation: commit,
            participants: self.participants.clone(),
            licence,
            term,
            quote: purchase.quote,
            quote_digest: purchase.quote_digest,
            product_digest: purchase.product_digest,
            holder: facts.requester(),
            issuer: facts.issuer(),
            charter: facts.charter(),
            line: facts.line(),
            filed: facts.filed(),
            revision: facts.revision(),
            rating: facts.rating(),
            terms: terms.terms,
            economics: HouseTermEconomicsV1::capture(
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
        super::admission_capacity::check(&f.house_economics, &row.economics)?;
        txn.replace(&f.deployment, &"registry".into(), deployment)
            .map_err(invalid)?;
        txn.insert(&f.house_allocations, &row.key(), row.clone())
            .map_err(invalid)?;
        if txn.get(&f.house_allocations, &row.key()).map_err(invalid)? != Some(&row) {
            return Err(PublicationError::Mismatch);
        }
        ctx.prepare_owner(
            &self.roots,
            RootRequest::House(Box::new(HouseRoots::funding(
                &self.action,
                &row,
                commit,
                facts,
                self.registration.1.clone(),
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
        before: &HouseAllocationV1,
        terms: &HousePurchaseTerms,
        observation: Option<&PreparedHouseSale>,
        output: &HouseTerminalOutput,
    ) -> Result<HouseAllocationV1, PublicationError> {
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
        let roots = HouseRoots::terminal(
            &self.action,
            requester,
            before,
            &after,
            output,
            facts,
            self.registration.1.clone(),
        )?;
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
                .get(&f.house_economics, &term_key)
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
            txn.insert(&f.paid_licences, &licence_key, licence.clone())
                .map_err(invalid)?;
            txn.insert(&f.paid_terms, &term_key, term.clone())
                .map_err(invalid)?;
            txn.insert(&f.house_economics, &term_key, before.economics.clone())
                .map_err(invalid)?;
            if txn.get(&f.paid_licences, &licence_key).map_err(invalid)? != Some(&licence)
                || txn.get(&f.paid_terms, &term_key).map_err(invalid)? != Some(&term)
                || txn.get(&f.house_economics, &term_key).map_err(invalid)?
                    != Some(&before.economics)
            {
                return Err(PublicationError::Mismatch);
            }
        }
        txn.replace(&f.house_allocations, &before.key(), after.clone())
            .map_err(invalid)?;
        if txn
            .get(&f.house_allocations, &before.key())
            .map_err(invalid)?
            != Some(&after)
        {
            return Err(PublicationError::Mismatch);
        }
        ctx.prepare_owner(&self.roots, RootRequest::House(Box::new(roots)))
            .map_err(invalid)?;
        Ok(after)
    }
}

fn check_purchase(
    p: &HousePurchase,
    terms: &HousePurchaseTerms,
    facts: &HouseSaleFacts,
) -> Result<(), PublicationError> {
    if p.buyer != facts.requester()
        || p.organization != facts.issuer().organization()
        || p.scope != facts.issuer().scope()
        || p.charter != facts.charter().object()
        || p.title_revision != facts.title_revision()
        || p.grant_economics_digest != terms.grant.digest()?
    {
        return Err(PublicationError::Mismatch);
    }
    Ok(())
}

//! Exact acknowledged personal paid holdings; no seller eligibility or IC claim.
//!
//! An opaque read observation is not authority to display source, mint a term,
//! transfer a licence, or install an IC sortie claim. Every OOC checkout rereads.
use super::families::CommerceFamilies;
use super::owner_holdings::OwnerProjectionError;
use super::rating::{RatingClass, RatingCompatibility, RatingFamilies, RatingRef};
use super::read::CommerceRead;
use super::records::{Record, RevisionPolicy, canonical_duplicate_digest, schema_line_digest};
use super::roots::{RequiredRoot, RootAnchor};
use super::{
    CharterId, FiledRevisionId, LicenceId, LineId, RevisionIdentity, TermGeneration, TermPhase,
    TermWindow, WorldIcSeconds,
};
use crate::{Revision, RollStore};
use mush_core::{DefinitionDigest, ObjId, ObjKind, StateFamilyHandle, flags::Flags};
use mush_economy::EconomyStore;

/// Internal source-free observation. Serialize is only for core result bounds;
/// there is no public constructor or Deserialize and it is never a public DTO.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct PersonalPaidHolding {
    actor: ObjId,
    charter: CharterId,
    operation: mush_core::CommitToken,
    terminal: mush_core::CommitToken,
    #[serde(skip_serializing_if = "Option::is_none")]
    holder_basis: Option<mush_economy::managed_paid_resale::RenewalHolderBasisV1>,
    terms: super::paid::PaidTerms,
    line: LineId,
    filed: FiledRevisionId,
    sequence: u64,
    licence: LicenceId,
    term: TermGeneration,
    window: TermWindow,
    name: String,
    revision: Revision,
    identity: RevisionIdentity,
    definition: DefinitionDigest,
    rating: RatingRef,
    compatibility: RatingCompatibility,
    declared: RatingClass,
    minimum: RatingClass,
}
impl PersonalPaidHolding {
    pub const fn actor(&self) -> ObjId {
        self.actor
    }
    pub const fn charter(&self) -> CharterId {
        self.charter
    }
    pub const fn operation(&self) -> mush_core::CommitToken {
        self.operation
    }
    pub const fn terminal(&self) -> mush_core::CommitToken {
        self.terminal
    }
    pub(crate) fn holder_basis(
        &self,
    ) -> Option<&mush_economy::managed_paid_resale::RenewalHolderBasisV1> {
        self.holder_basis.as_ref()
    }
    pub const fn terms(&self) -> super::paid::PaidTerms {
        self.terms
    }
    pub const fn line(&self) -> LineId {
        self.line
    }
    pub const fn filed(&self) -> FiledRevisionId {
        self.filed
    }
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }
    pub const fn licence(&self) -> LicenceId {
        self.licence
    }
    pub const fn term(&self) -> TermGeneration {
        self.term
    }
    pub const fn window(&self) -> TermWindow {
        self.window
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn revision(&self) -> &Revision {
        &self.revision
    }
    pub const fn identity(&self) -> RevisionIdentity {
        self.identity
    }
    pub const fn definition(&self) -> DefinitionDigest {
        self.definition
    }
    pub const fn rating(&self) -> RatingRef {
        self.rating
    }
    pub fn compatibility(&self) -> &RatingCompatibility {
        &self.compatibility
    }
    pub const fn declared(&self) -> RatingClass {
        self.declared
    }
    pub const fn minimum(&self) -> RatingClass {
        self.minimum
    }
}

pub(crate) fn project_personal_paid_for(
    ctx: &impl CommerceRead,
    holder: ObjId,
    families: &CommerceFamilies,
    ratings: &RatingFamilies,
    licence_id: LicenceId,
    expected: &RatingCompatibility,
) -> Result<PersonalPaidHolding, OwnerProjectionError> {
    project_paid_for(
        ctx,
        holder,
        families,
        ratings,
        licence_id,
        expected,
        Scope::Original,
    )
}

/// Only separately registered entitlement readers select this path. Existing
/// original-only source, fork-parent and holding descriptors keep their scope.
pub(crate) fn project_entitlement_paid_for(
    ctx: &impl CommerceRead,
    holder: ObjId,
    families: &CommerceFamilies,
    ratings: &RatingFamilies,
    licence_id: LicenceId,
    expected: &RatingCompatibility,
) -> Result<PersonalPaidHolding, OwnerProjectionError> {
    project_paid_for(
        ctx,
        holder,
        families,
        ratings,
        licence_id,
        expected,
        Scope::Entitlement,
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    Original,
    Entitlement,
    House,
}
pub(crate) fn project_house_paid_for(
    ctx: &impl CommerceRead,
    holder: ObjId,
    families: &CommerceFamilies,
    ratings: &RatingFamilies,
    licence_id: LicenceId,
    expected: &RatingCompatibility,
) -> Result<PersonalPaidHolding, OwnerProjectionError> {
    project_paid_for(
        ctx,
        holder,
        families,
        ratings,
        licence_id,
        expected,
        Scope::House,
    )
}

fn project_paid_for(
    ctx: &impl CommerceRead,
    holder: ObjId,
    families: &CommerceFamilies,
    ratings: &RatingFamilies,
    licence_id: LicenceId,
    expected: &RatingCompatibility,
    scope: Scope,
) -> Result<PersonalPaidHolding, OwnerProjectionError> {
    let roll = ctx
        .world()
        .module_state::<RollStore>()
        .ok_or(OwnerProjectionError::Unavailable)?;
    roll.commerce_ready()
        .map_err(|_| OwnerProjectionError::Unavailable)?;
    if ctx
        .world()
        .get(holder)
        .is_none_or(|object| object.kind != ObjKind::Player || object.flags.contains(Flags::GOING))
    {
        return Err(OwnerProjectionError::NotHeld);
    }
    let economy = ctx
        .world()
        .module_state::<EconomyStore>()
        .ok_or(OwnerProjectionError::Unavailable)?;
    if economy.is_frozen() {
        return Err(OwnerProjectionError::Unavailable);
    }
    let history = economy.purchase_receipt_audit();
    history
        .require_complete()
        .map_err(|_| OwnerProjectionError::Unavailable)?;
    let licence = read(ctx, &families.paid_licences, licence_id.key())?;
    if licence.holder() != holder {
        return Err(OwnerProjectionError::NotHeld);
    }
    let term = read(
        ctx,
        &families.paid_terms,
        format!("{}/{}", licence.id().key(), licence.term().key()),
    )?;
    let allocation = super::entitlement::history::current(ctx, families, ratings, licence, term)
        .map_err(|_| OwnerProjectionError::Integrity)?;
    super::registry_revocation::current::require(
        ctx,
        families,
        licence.id(),
        term.generation(),
        None,
    )
    .map_err(|_| OwnerProjectionError::NotHeld)?;
    if matches!(
        allocation.birth(),
        super::entitlement::history::Birth::House(_)
    ) != (scope == Scope::House)
    {
        return Err(OwnerProjectionError::NotHeld);
    }
    let terminal = allocation.terminal();
    let now = WorldIcSeconds::from_stamp(&ctx.world().world_ic_now())
        .map_err(|_| OwnerProjectionError::Integrity)?;
    if !matches!(
        term.window().phase_at(now),
        TermPhase::Active | TermPhase::Grace
    ) {
        return Err(OwnerProjectionError::NotHeld);
    }
    let RevisionPolicy::Pin {
        filed: filed_id,
        revision: identity,
    } = term.policy()
    else {
        // Current paid writers and boot support exact personal Pin only.
        // Future Follow requires its own bounded eligible-line resolution.
        return Err(OwnerProjectionError::Integrity);
    };
    let charter = read(ctx, &families.charters, licence.charter().key())?;
    let line = read(ctx, &families.lines, licence.line().key())?;
    let filed = read(ctx, &families.filed, filed_id.key())?;
    let status = read(ctx, &families.status, filed_id.key())?;
    if (scope != Scope::Entitlement && filed.content.parent().is_some())
        || status.filed != filed_id
        || charter.line != line.id
        || line.charter != charter.id
        || filed.content.position().charter != charter.id
        || filed.content.position().line != line.id
        || filed.content.revision() != identity
    {
        return Err(OwnerProjectionError::Integrity);
    }
    match allocation.birth() {
        super::entitlement::history::Birth::House(_)
            if scope == Scope::House && filed.content.parent().is_none() => {}
        super::entitlement::history::Birth::Legacy(_) if filed.content.parent().is_none() => {}
        super::entitlement::history::Birth::Entitlement(row) => {
            let original = row.protocol()
                == mush_economy::managed_entitlement::EntitlementProtocol::OriginalV3;
            if original != filed.content.parent().is_none() {
                return Err(OwnerProjectionError::Integrity);
            }
            // Exact immutable chain consent, without checking an issuer's
            // current parent term, title, source access or ancestor standing.
            let chain = read(ctx, &families.filed_royalties, filed_id.key())?;
            if chain.revision() != identity
                || chain
                    .reference()
                    .map_err(|_| OwnerProjectionError::Integrity)?
                    != row.economics().grant().royalty_chain()
            {
                return Err(OwnerProjectionError::Integrity);
            }
        }
        _ => return Err(OwnerProjectionError::Integrity),
    }
    super::rating::reconformance::history::certificate(ctx, ratings, filed, &allocation.rating())
        .map_err(|_| OwnerProjectionError::Integrity)?;
    // Existing Pin remains valid after withdrawal. Recall applies to this exact
    // selected filed revision; no live issuer/ancestor grant is consulted.
    if status.recalled_by.is_some() {
        return Err(OwnerProjectionError::NotHeld);
    }
    let rating =
        super::rating::reconformance::current::receipt(ctx, families, ratings, filed, expected)
            .map_err(|_| OwnerProjectionError::NotHeld)?;
    if rating.subject().revision() != identity
        || rating.subject().definition() != filed.definition
        || rating.subject().schema() != line.schema_digest
    {
        return Err(OwnerProjectionError::Integrity);
    }
    // The newly measured minimum cannot enlarge an already accepted paid class.
    if term.terms().class() < rating.minimum() {
        return Err(OwnerProjectionError::NotHeld);
    }
    for required in [
        RequiredRoot {
            anchor: RootAnchor::Filed(filed_id),
            identity,
        },
        RequiredRoot {
            anchor: RootAnchor::Rating {
                job: rating.publication().job(),
                id: rating.id(),
            },
            identity,
        },
        RequiredRoot {
            anchor: RootAnchor::PaidOperation(allocation.birth_operation()),
            identity,
        },
        RequiredRoot {
            anchor: RootAnchor::Term(licence.id(), term.generation()),
            identity,
        },
    ] {
        roll.check_required_root(&required)
            .map_err(|_| OwnerProjectionError::Integrity)?;
    }
    if allocation.operation() != allocation.birth_operation() {
        // A renewed generation's root is its renewal operation, House or personal.
        let anchor = match allocation.birth() {
            super::entitlement::history::Birth::House(_) => {
                RootAnchor::HouseRenewalOperation(allocation.operation())
            }
            _ => RootAnchor::RenewalOperation(allocation.operation()),
        };
        roll.check_required_root(&RequiredRoot { anchor, identity })
            .map_err(|_| OwnerProjectionError::Integrity)?;
    }
    let reference = identity.revision();
    let revision = roll
        .revision(reference.key.character, reference.key)
        .map_err(|_| OwnerProjectionError::NotHeld)?;
    if !identity
        .matches(revision)
        .map_err(|_| OwnerProjectionError::Integrity)?
        || schema_line_digest(revision.envelope()).map_err(|_| OwnerProjectionError::Integrity)?
            != line.schema_digest
        || canonical_duplicate_digest(revision).map_err(|_| OwnerProjectionError::Integrity)?
            != filed.duplicate
    {
        return Err(OwnerProjectionError::Integrity);
    }
    // `current` above already proved the complete native/typed/root ownership
    // history. Capture that same immutable holder row without repeating the
    // certificate/page history inside this bounded owner-read observation.
    let holder_basis = match licence.ownership() {
        None => None,
        Some(ownership) => {
            let row = read(
                ctx,
                &families.paid_holders,
                super::paid_resale::PaidHolderRecord::key_for(licence.id(), ownership.generation),
            )?;
            if row
                .ownership()
                .map_err(|_| OwnerProjectionError::Integrity)?
                != *ownership
                || row.licence() != licence.id()
                || row.holder() != holder
            {
                return Err(OwnerProjectionError::Integrity);
            }
            let transfer = read(ctx, &families.paid_resales, row.acceptance().to_hex())?;
            row.matches_record(transfer)
                .map_err(|_| OwnerProjectionError::Integrity)?;
            let settled = transfer
                .economic()
                .terminal()
                .map(|t| &t.decision().at_ic)
                .unwrap_or(&transfer.economic().consent().accepted_at_ic);
            let observed = ctx.world().world_ic_now();
            if settled.clock == observed.clock && settled.at > observed.at {
                return Err(OwnerProjectionError::NotHeld);
            }
            Some(mush_economy::managed_paid_resale::RenewalHolderBasisV1 {
                acceptance: row.acceptance(),
                ownership: *ownership,
            })
        }
    };
    Ok(PersonalPaidHolding {
        actor: holder,
        charter: charter.id,
        operation: allocation.operation(),
        terminal,
        holder_basis,
        terms: term.terms(),
        line: line.id,
        filed: filed_id,
        sequence: filed.content.position().sequence.get(),
        licence: licence.id(),
        term: term.generation(),
        window: term.window(),
        name: charter.name.clone(),
        revision: revision.clone(),
        identity,
        definition: filed.definition,
        rating: rating
            .reference()
            .map_err(|_| OwnerProjectionError::Integrity)?,
        compatibility: rating.compatibility().clone(),
        declared: rating.subject().declared(),
        minimum: rating.minimum(),
    })
}

fn read<'a, R: Record>(
    ctx: &'a impl CommerceRead,
    handle: &StateFamilyHandle<String, R>,
    key: String,
) -> Result<&'a R, OwnerProjectionError> {
    let row = ctx
        .get(handle, &key)
        .map_err(|_| OwnerProjectionError::Unavailable)?
        .ok_or(OwnerProjectionError::NotHeld)?;
    row.validate()
        .map_err(|_| OwnerProjectionError::Integrity)?;
    if row.key() != key {
        return Err(OwnerProjectionError::Integrity);
    }
    Ok(row)
}

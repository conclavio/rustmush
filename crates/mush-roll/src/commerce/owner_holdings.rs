//! Original character-owner selection/source projection over real committed rows.
//!
//! The resolver is Roll-private and shared by the actual owner boot context and
//! the bounded registered `holding_reader` service. Both carry an owner lens. A
//! projection is a point-in-time read, never a claim, dispatch warrant, current
//! compatibility declaration, or permission to install an IC seal. Checkout
//! must resolve again; an IC checkout additionally commits its exact open claim.

use mush_core::{DefinitionDigest, ObjId, StateFamilyHandle};
use mush_economy::{AssetLifecycle, EconomicAsset, EconomyStore};
use mush_org::PrincipalId;

use super::families::CommerceFamilies;
use super::rating::{RatingClass, RatingCompatibility, RatingRef, families::RatingFamilies};
use super::read::CommerceRead;
use super::records::{Record, RevisionPolicy, canonical_duplicate_digest, schema_line_digest};
use super::roots::{RequiredRoot, RootAnchor};
use super::{
    CharterId, FiledRevisionId, LicenceId, LineId, RevisionIdentity, TermGeneration, TermPhase,
    TermWindow, WorldIcSeconds,
};
use crate::{Revision, RollStore};

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum OwnerProjectionError {
    #[error("owned mind records are unavailable")]
    Unavailable,
    #[error("no current original-owner holding")]
    NotHeld,
    #[error("owned mind records disagree")]
    Integrity,
}

/// Source-free executable metadata and exact observed rights. There is no
/// Deserialize/public constructor, mutable family handle, or cached owner map.
/// Serialize is used only to enforce the core internal result-size bound.
/// Consumers must not serialize this internal projection into a public listing.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct OriginalOwnerHolding {
    actor: ObjId,
    /// M8c B2: whom the title is held for: the actor itself, or the
    /// organisation the actor acts for (`check_title`'s organisation arm).
    holder: super::Holder,
    ownership: super::charter_transfer::records::CharterOwnershipRef,
    charter: CharterId,
    title_revision: u64,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    qualification: Option<super::owner_qualification::current::CurrentQualification>,
    declared: RatingClass,
    minimum: RatingClass,
}
impl OriginalOwnerHolding {
    pub(crate) fn qualification(
        &self,
    ) -> Option<&super::owner_qualification::current::CurrentQualification> {
        self.qualification.as_ref()
    }
    pub(crate) fn ownership(&self) -> &super::charter_transfer::records::CharterOwnershipRef {
        &self.ownership
    }
    /// M8c B2: the holder the title projects as (`check_title`).
    pub const fn holder(&self) -> super::Holder {
        self.holder
    }
    pub const fn actor(&self) -> ObjId {
        self.actor
    }
    pub const fn charter(&self) -> CharterId {
        self.charter
    }
    pub const fn title_revision(&self) -> u64 {
        self.title_revision
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

/// `expected` must come from the registered game's current manifest, not a
/// transport field. This stays crate-private behind the registered reader and
/// the closed measured Registry completion producer. M8c B2: an organisation's
/// title is admitted for a character acting for the organisation on the
/// title's scope under a live `roll.holder.org` grant
/// (`org_holder::scoped_actor`), and the holding then projects as
/// `Holder::Organization { org, acting }`; staff and assignment never
/// substitute for title ownership, nor for the grant.
/// Admission may inspect another real selected pilot/lead. The holder is a
/// separate subject, never a substituted requester identity or owner lens.
pub(crate) fn project_original_owner_for(
    ctx: &impl CommerceRead,
    holder: ObjId,
    families: &CommerceFamilies,
    ratings: &RatingFamilies,
    licence_id: LicenceId,
    expected: &RatingCompatibility,
) -> Result<OriginalOwnerHolding, OwnerProjectionError> {
    project_owner_for(ctx, holder, families, ratings, licence_id, expected, false)
}

/// Closed dependent-capable reader only. Original sale and original holding
/// services continue calling the original-only wrapper above.
pub(crate) fn project_owner_with_parent_for(
    ctx: &impl CommerceRead,
    holder: ObjId,
    families: &CommerceFamilies,
    ratings: &RatingFamilies,
    licence_id: LicenceId,
    expected: &RatingCompatibility,
) -> Result<OriginalOwnerHolding, OwnerProjectionError> {
    project_owner_for(ctx, holder, families, ratings, licence_id, expected, true)
}

fn project_owner_for(
    ctx: &impl CommerceRead,
    holder: ObjId,
    families: &CommerceFamilies,
    ratings: &RatingFamilies,
    licence_id: LicenceId,
    expected: &RatingCompatibility,
    dependent: bool,
) -> Result<OriginalOwnerHolding, OwnerProjectionError> {
    let roll = ctx
        .world()
        .module_state::<RollStore>()
        .ok_or(OwnerProjectionError::Unavailable)?;
    roll.commerce_ready()
        .map_err(|_| OwnerProjectionError::Unavailable)?;
    let economy = ctx
        .world()
        .module_state::<EconomyStore>()
        .ok_or(OwnerProjectionError::Unavailable)?;
    if economy.is_frozen() {
        return Err(OwnerProjectionError::Unavailable);
    }
    let licence = read(ctx, &families.licences, licence_id.key())?;
    let charter = read(ctx, &families.charters, licence.charter.key())?;
    let line = read(ctx, &families.lines, licence.line.key())?;
    let term = super::owner_attestation::history::current(ctx, roll, families, licence)
        .map_err(|_| OwnerProjectionError::Integrity)?;
    let title = economy
        .snapshot()
        .asset(ObjId(charter.id.get()))
        .ok_or(OwnerProjectionError::NotHeld)?;
    let held_as = check_title(holder, charter.id, title, |org, scope| {
        super::org_holder::scoped_actor(ctx.world(), org, scope, holder).is_ok()
    })?;
    // Doc 17h cut M8d D4: a principal the Registry revoked flies, seals and reads no held term
    // while the window runs; the term itself is untouched and resumes after it.
    if super::registry_inspection::principal::barred_read(ctx, families, held_as)
        .map_err(|_| OwnerProjectionError::Unavailable)?
    {
        return Err(OwnerProjectionError::NotHeld);
    }
    let birth = read(ctx, &families.operations, licence.operation.to_hex())?;
    let ownership = super::charter_transfer::history::current_for(ctx, families, birth, held_as)
        .map_err(|_| OwnerProjectionError::NotHeld)?;
    check_term(term.window, WorldIcSeconds(ctx.world().game_now_unix()))?;
    super::registry_revocation::current::require(ctx, families, licence.id, term.generation, None)
        .map_err(|_| OwnerProjectionError::NotHeld)?;
    if charter.line != line.id
        || line.charter != charter.id
        || licence.charter != charter.id
        || licence.line != line.id
        || term.licence != licence.id
        || term.generation != licence.term
        || [charter.operation, line.operation]
            .into_iter()
            .any(|op| op != licence.operation)
    {
        return Err(OwnerProjectionError::Integrity);
    }
    let (filed_id, pin) = match term.revision {
        RevisionPolicy::Pin { filed, revision } => (filed, Some(revision)),
        RevisionPolicy::Follow { line: selected } if selected == line.id => (
            roll.follow_index()
                .map_err(|_| OwnerProjectionError::Unavailable)?
                .select(line, expected, WorldIcSeconds(ctx.world().game_now_unix()))
                .map_err(|error| match error {
                    super::records::PublicationError::Unavailable => OwnerProjectionError::NotHeld,
                    _ => OwnerProjectionError::Integrity,
                })?,
            None,
        ),
        RevisionPolicy::Follow { .. } => return Err(OwnerProjectionError::Integrity),
    };
    let filed = read(ctx, &families.filed, filed_id.key())?;
    let status = read(ctx, &families.status, filed_id.key())?;
    check_status(
        pin.is_some(),
        status.withdrawn_by.is_some(),
        status.recalled_by.is_some(),
    )?;
    let identity = filed.content.revision();
    if filed.content.position().charter != charter.id
        || filed.content.position().line != line.id
        || (!matches!(filed.format, 3 | 4) && filed.operation != licence.operation)
        || status.filed != filed_id
        || pin.is_some_and(|p| p != identity)
    {
        return Err(OwnerProjectionError::Integrity);
    }
    let qualification = if filed.content.parent().is_some() {
        if !dependent {
            return Err(OwnerProjectionError::NotHeld);
        }
        let binding = read(
            ctx,
            &families.owner_parents,
            super::dependent_owner::OwnerParentBinding::key_for(licence.id, birth.requester),
        )?;
        let operation = read(ctx, &families.operations, licence.operation.to_string())?;
        let birth_term = read(
            ctx,
            &families.terms,
            super::records::OwnerTerm::encoded_key(licence.id, binding.term),
        )?;
        let birth_filed = read(ctx, &families.filed, binding.filed.key())?;
        if filed.content.parent() != Some(binding.parent.parent) {
            return Err(OwnerProjectionError::Integrity);
        }
        super::dependent_owner::verify_history(
            ctx,
            roll,
            families,
            ratings,
            binding,
            birth_filed,
            birth_term,
            operation,
        )
        .map_err(|_| OwnerProjectionError::Integrity)?;
        Some(super::dependent_owner::require_current(
            ctx, holder, families, ratings, expected, binding,
        )?)
    } else {
        if !matches!(filed.format, 1 | 3) || !matches!(term.format, 1 | 3) || term.parent.is_some()
        {
            return Err(OwnerProjectionError::Integrity);
        }
        None
    };
    if matches!(filed.format, 3 | 4) {
        let birth = super::line_update::audit::birth(ctx, families, filed)
            .map_err(|_| OwnerProjectionError::Integrity)?;
        if birth.commit != licence.operation {
            return Err(OwnerProjectionError::Integrity);
        }
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
    if pin.is_none() {
        roll.follow_index()
            .map_err(|_| OwnerProjectionError::Unavailable)?
            .check_selected(
                filed,
                expected,
                &rating
                    .reference()
                    .map_err(|_| OwnerProjectionError::Integrity)?,
            )
            .map_err(|_| OwnerProjectionError::Integrity)?;
    }
    roll.check_required_root(&RequiredRoot {
        anchor: RootAnchor::Filed(filed_id),
        identity,
    })
    .map_err(|_| OwnerProjectionError::Integrity)?;
    roll.check_required_root(&RequiredRoot {
        anchor: RootAnchor::Rating {
            job: rating.publication().job(),
            id: rating.id(),
        },
        identity,
    })
    .map_err(|_| OwnerProjectionError::Integrity)?;
    if pin.is_some() {
        roll.check_required_root(&RequiredRoot {
            anchor: RootAnchor::Term(licence.id, term.generation),
            identity,
        })
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
    Ok(OriginalOwnerHolding {
        actor: holder,
        holder: held_as,
        ownership,
        charter: charter.id,
        title_revision: title.revision,
        line: line.id,
        filed: filed_id,
        sequence: filed.content.position().sequence.get(),
        licence: licence.id,
        term: term.generation,
        window: term.window,
        name: charter.name.clone(),
        revision: revision.clone(),
        identity,
        definition: filed.definition,
        rating: rating
            .reference()
            .map_err(|_| OwnerProjectionError::Integrity)?,
        compatibility: rating.compatibility().clone(),
        qualification,
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
/// The title check, pure: the charter's current E4 title is the actor's own
/// (`Holder::Character`), or (M8c B2) an organisation's for which
/// `org_actor(org, title.scope)` answers true, the live `roll.holder.org`
/// grant asked by the caller (`Holder::Organization { org, acting: actor }`).
/// A frozen or inactive title is refused on either arm; staff and assignment
/// never substitute for title.
fn check_title(
    actor: ObjId,
    charter: CharterId,
    title: &EconomicAsset,
    org_actor: impl Fn(mush_org::OrgId, Option<mush_org::OrgScopeId>) -> bool,
) -> Result<super::Holder, OwnerProjectionError> {
    if actor == ObjId(0)
        || title.object != ObjId(charter.get())
        || title.revision == 0
        || title.frozen
        || title.lifecycle != AssetLifecycle::Active
    {
        return Err(OwnerProjectionError::NotHeld);
    }
    match title.title_owner {
        PrincipalId::Object(owner) if owner == actor => Ok(super::Holder::Character(actor)),
        PrincipalId::Organization(org) if org_actor(org, title.scope) => {
            Ok(super::Holder::Organization { org, acting: actor })
        }
        _ => Err(OwnerProjectionError::NotHeld),
    }
}
fn check_term(window: TermWindow, now: WorldIcSeconds) -> Result<(), OwnerProjectionError> {
    match window.phase_at(now) {
        TermPhase::Active | TermPhase::Grace => Ok(()),
        TermPhase::NotEffective | TermPhase::Lapsed => Err(OwnerProjectionError::NotHeld),
    }
}
fn check_status(pinned: bool, withdrawn: bool, recalled: bool) -> Result<(), OwnerProjectionError> {
    if recalled || (withdrawn && !pinned) {
        return Err(OwnerProjectionError::NotHeld);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn title(owner: ObjId) -> EconomicAsset {
        EconomicAsset {
            object: ObjId(21),
            title_owner: PrincipalId::Object(owner),
            scope: None,
            assignment: None,
            maintenance_account: None,
            catalog: None,
            encumbrances: vec![],
            acquisition: None,
            lifecycle: AssetLifecycle::Active,
            last_journal_seq: 1,
            revision: 1,
            stolen: None,
            frozen: false,
            frozen_reason: None,
        }
    }
    /// A character's own title never asks the organisation predicate.
    fn never(_: mush_org::OrgId, _: Option<mush_org::OrgScopeId>) -> bool {
        panic!("a character title asked the organisation predicate")
    }
    #[test]
    fn title_is_current_e4_title_not_source_author_or_staff_or_assignment() {
        let charter = CharterId::new(21).unwrap();
        let mut row = title(ObjId(7));
        assert_eq!(
            check_title(ObjId(7), charter, &row, never).ok(),
            Some(super::super::Holder::Character(ObjId(7)))
        );
        assert!(check_title(ObjId(8), charter, &row, never).is_err());
        row.title_owner = PrincipalId::Object(ObjId(8));
        row.revision += 1;
        assert!(check_title(ObjId(7), charter, &row, never).is_err());
        assert!(check_title(ObjId(8), charter, &row, never).is_ok());
        // M8c B2: an organisation's title is refused without the grant...
        row.title_owner = PrincipalId::Organization(mush_org::OrgId(1));
        assert!(check_title(ObjId(8), charter, &row, |_, _| false).is_err());
    }
    /// M8c B2 (the brief's test 5): the organisation arm admits exactly the
    /// character the live grant answers for, on the title's own scope, and
    /// projects the holding as the organisation acting through that
    /// character; the title's integrity checks still refuse first.
    #[test]
    fn organisation_title_admits_a_granted_actor_on_the_titles_scope() {
        use mush_org::{OrgId, OrgScopeId};
        let charter = CharterId::new(21).unwrap();
        let mut row = title(ObjId(7));
        row.title_owner = PrincipalId::Organization(OrgId(1));
        row.scope = Some(OrgScopeId(5));
        let asked = std::cell::RefCell::new(Vec::new());
        let granted = |org: OrgId, scope: Option<OrgScopeId>| {
            asked.borrow_mut().push((org, scope));
            org == OrgId(1) && scope == Some(OrgScopeId(5))
        };
        assert_eq!(
            check_title(ObjId(8), charter, &row, granted).ok(),
            Some(super::super::Holder::Organization {
                org: OrgId(1),
                acting: ObjId(8),
            })
        );
        assert_eq!(
            asked.borrow().as_slice(),
            &[(OrgId(1), Some(OrgScopeId(5)))]
        );
        // The predicate's refusal is the title's refusal: no other arm.
        assert!(check_title(ObjId(8), charter, &row, |_, _| false).is_err());
        // An unscoped organisation title asks for the root scope.
        row.scope = None;
        assert_eq!(
            check_title(ObjId(9), charter, &row, |org, scope| org == OrgId(1)
                && scope.is_none())
            .ok(),
            Some(super::super::Holder::Organization {
                org: OrgId(1),
                acting: ObjId(9),
            })
        );
        // Integrity and lifecycle refuse before the grant is asked.
        row.frozen = true;
        assert!(check_title(ObjId(8), charter, &row, never).is_err());
        row.frozen = false;
        row.lifecycle = AssetLifecycle::Abandoned;
        assert!(check_title(ObjId(8), charter, &row, never).is_err());
        row.lifecycle = AssetLifecycle::Active;
        assert!(check_title(ObjId(0), charter, &row, never).is_err());
    }
    #[test]
    fn title_integrity_and_lifecycle_do_not_silently_fall_back_to_the_author() {
        let charter = CharterId::new(21).unwrap();
        let mut row = title(ObjId(7));
        for lifecycle in [AssetLifecycle::WrittenOff, AssetLifecycle::Abandoned] {
            row.lifecycle = lifecycle;
            assert!(check_title(ObjId(7), charter, &row, never).is_err());
        }
        row.lifecycle = AssetLifecycle::Active;
        row.frozen = true;
        assert!(check_title(ObjId(7), charter, &row, never).is_err());
        row.frozen = false;
        row.object = ObjId(22);
        assert!(check_title(ObjId(7), charter, &row, never).is_err());
    }
    #[test]
    fn term_uses_world_ic_and_never_extends_at_grace_or_lapse_boundary() {
        let w = TermWindow::new(
            WorldIcSeconds(100),
            WorldIcSeconds(90),
            WorldIcSeconds(200),
            WorldIcSeconds(220),
        )
        .unwrap();
        for now in [90, 99, 220, 221] {
            assert!(check_term(w, WorldIcSeconds(now)).is_err());
        }
        for now in [100, 199, 200, 219] {
            assert!(check_term(w, WorldIcSeconds(now)).is_ok());
        }
        assert_eq!(w.term_end(), WorldIcSeconds(200));
        assert_eq!(w.grace_end(), WorldIcSeconds(220));
    }
    #[test]
    fn withdrawal_keeps_existing_pins_but_recall_refuses_both_selection_modes() {
        assert!(check_status(true, true, false).is_ok());
        assert!(check_status(false, true, false).is_err());
        for pinned in [false, true] {
            assert!(check_status(pinned, false, true).is_err());
            assert!(check_status(pinned, true, true).is_err());
            assert!(check_status(pinned, false, false).is_ok());
        }
    }
}

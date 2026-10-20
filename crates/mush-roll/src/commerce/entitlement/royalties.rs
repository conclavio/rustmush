//! Immutable economics follow the actual source acquisition, not whichever
//! current grant later qualifies a sale. No current title, time or parent
//! eligibility is consulted here; the separate sale reader must prove those.

use mush_core::CommitToken;
use mush_economy::{EconomyStore, managed_entitlement::EntitlementProtocol};
use serde::Serialize;

use super::super::FiledRevisionId;
use super::super::dependent_owner::ContentView;
use super::super::families::CommerceFamilies;
use super::super::paid::PaidForm;
use super::super::parent_selector::DraftParentSelection;
use super::super::rating::RatingFamilies;
use super::super::read::CommerceRead;
use super::super::records::{FiledRecord, PublicationError, Record, RevisionPolicy};
use super::super::roots::{RequiredRoot, RootAnchor};
use super::history::{self, Birth};
use super::{AcquisitionV1, FiledRoyaltyTermsV1, RoyaltyBasisV1, RoyaltyEdgeV1};
use crate::RollStore;

/// Private derivation result. Only the closed Roll writer may stage these
/// exact missing rows in the same listing transaction. Existing rows never
/// become replacements, even when a subsequent listing has another token.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct RoyaltyMaterialization {
    primary: FiledRoyaltyTermsV1,
    creates: Vec<FiledRoyaltyTermsV1>,
}
impl RoyaltyMaterialization {
    pub(crate) fn primary(&self) -> &FiledRoyaltyTermsV1 {
        &self.primary
    }
    pub(crate) fn creates(&self) -> &[FiledRoyaltyTermsV1] {
        &self.creates
    }
}

fn mismatch(_: impl std::fmt::Display) -> PublicationError {
    PublicationError::Mismatch
}

fn exact_filed<'a>(
    ctx: &'a impl CommerceRead,
    roll: &RollStore,
    f: &CommerceFamilies,
    id: FiledRevisionId,
) -> Result<&'a FiledRecord, PublicationError> {
    let filed = history::read(ctx, &f.filed, id.key())?;
    filed
        .content
        .check_links(&ContentView {
            ctx,
            roll,
            families: f,
        })
        .map_err(mismatch)?;
    roll.check_required_root(&RequiredRoot {
        anchor: RootAnchor::Filed(id),
        identity: filed.content.revision(),
    })
    .map_err(mismatch)?;
    Ok(filed)
}

fn original_chain(
    ctx: &impl CommerceRead,
    roll: &RollStore,
    f: &CommerceFamilies,
    filed: &FiledRecord,
    publication: CommitToken,
    creates: &mut Vec<FiledRoyaltyTermsV1>,
) -> Result<FiledRoyaltyTermsV1, PublicationError> {
    let revision = roll
        .exact(filed.content.revision().revision())
        .map_err(mismatch)?;
    if filed.content.parent().is_some() || revision.provenance.is_some() {
        return Err(PublicationError::Mismatch);
    }
    capture_or_match(ctx, f, filed, publication, Vec::new(), creates)
}

fn capture_or_match(
    ctx: &impl CommerceRead,
    f: &CommerceFamilies,
    filed: &FiledRecord,
    publication: CommitToken,
    edges: Vec<RoyaltyEdgeV1>,
    creates: &mut Vec<FiledRoyaltyTermsV1>,
) -> Result<FiledRoyaltyTermsV1, PublicationError> {
    let existing = history::optional(ctx, &f.filed_royalties, filed.content.id().key())?;
    let expected = FiledRoyaltyTermsV1::capture(
        filed.content.id(),
        filed.content.revision(),
        existing.map_or(publication, FiledRoyaltyTermsV1::publication),
        edges,
    )?;
    match existing {
        Some(row) if row == &expected => Ok(row.clone()),
        Some(_) => Err(PublicationError::Mismatch),
        None => {
            if creates.len() >= super::super::filed::MAX_PARENT_DEPTH + 1 {
                return Err(PublicationError::Capacity);
            }
            creates.push(expected.clone());
            Ok(expected)
        }
    }
}

/// Immutable acquisition is resolved independently at every retained parent.
/// This does not enable a deeper fork/filing producer or require an ancestor's
/// current rights. Every actual term.operation and accepted chain is proved.
pub(crate) fn prepare(
    ctx: &impl CommerceRead,
    roll: &RollStore,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
    filed_id: FiledRevisionId,
    publication: CommitToken,
) -> Result<RoyaltyMaterialization, PublicationError> {
    let mut creates = Vec::with_capacity(super::super::filed::MAX_PARENT_DEPTH + 1);
    let primary = prepare_depth(
        ctx,
        roll,
        f,
        ratings,
        filed_id,
        publication,
        0,
        &mut creates,
    )?;
    Ok(RoyaltyMaterialization { primary, creates })
}

#[allow(clippy::too_many_arguments)]
fn prepare_depth(
    ctx: &impl CommerceRead,
    roll: &RollStore,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
    filed_id: FiledRevisionId,
    publication: CommitToken,
    depth: usize,
    creates: &mut Vec<FiledRoyaltyTermsV1>,
) -> Result<FiledRoyaltyTermsV1, PublicationError> {
    if depth > super::super::filed::MAX_PARENT_DEPTH {
        return Err(PublicationError::Capacity);
    }
    let filed = exact_filed(ctx, roll, f, filed_id)?;
    let Some(parent) = filed.content.parent() else {
        return original_chain(ctx, roll, f, filed, publication, creates);
    };
    let parent_row = exact_filed(ctx, roll, f, parent.filed)?;
    if parent_row.content.revision() != parent.revision {
        return Err(PublicationError::Mismatch);
    }
    let revision = roll
        .exact(filed.content.revision().revision())
        .map_err(mismatch)?;
    let key = revision.reference.key;
    let selector = history::read(
        ctx,
        &f.draft_parents,
        DraftParentSelection::key_for(key.character, key.mind),
    )?;
    // The live draft may have been retired. The immutable saved revision and
    // retained acquisition selector remain the historical evidence; this is
    // never a source or authoring access decision.
    if selector.actor != key.character
        || selector.mind != key.mind
        || selector.parent != parent
        || revision.provenance.as_ref() != Some(&selector.provenance())
        || selector.charter != parent_row.content.position().charter
        || selector.line != parent_row.content.position().line
    {
        return Err(PublicationError::Mismatch);
    }
    let licence = history::read(ctx, &f.paid_licences, selector.licence.key())?;
    let term = history::read(
        ctx,
        &f.paid_terms,
        format!(
            "{}/{}",
            selector.licence.key(),
            selector.created_parent_term.key()
        ),
    )?;
    let birth = history::granted(ctx, f, ratings, licence, term)?;
    super::super::owner_qualification::history::require_captured_holder(
        ctx,
        f,
        ratings,
        licence,
        selector.holder_capture.as_ref(),
        birth.holder(),
        selector.actor,
    )?;
    if birth.licence() != selector.licence
        || birth.term() != selector.created_parent_term
        || birth.charter() != selector.charter
        || birth.line() != selector.line
        || birth.filed() != parent.filed
        || birth.revision() != parent.revision
        || term.policy()
            != (RevisionPolicy::Pin {
                filed: parent.filed,
                revision: parent.revision,
            })
        || term.terms().form() != (PaidForm::OpenResale {})
    {
        return Err(PublicationError::Mismatch);
    }
    let had_parent_chain =
        history::optional(ctx, &f.filed_royalties, parent.filed.key())?.is_some();
    let parent_chain = prepare_depth(
        ctx,
        roll,
        f,
        ratings,
        parent.filed,
        publication,
        depth + 1,
        creates,
    )?;
    let (royalty_permille, basis) = match birth.birth() {
        Birth::House(_) => return Err(PublicationError::Mismatch),
        Birth::Legacy(row) => {
            if row.format != 2 || parent_row.content.parent().is_some() {
                return Err(PublicationError::Mismatch);
            }
            let economy = ctx
                .world()
                .module_state::<EconomyStore>()
                .ok_or(PublicationError::Unavailable)?;
            let actual = economy
                .purchase_receipt_audit()
                .get(row.operation)
                .map_err(mismatch)?;
            (
                0,
                RoyaltyBasisV1::LegacySourceFormsV2Zero {
                    effect: actual.effect().clone(),
                },
            )
        }
        Birth::Entitlement(row) => {
            // A new original grant was consented against an already committed
            // exact chain. Missing it is corruption, never a legacy conversion.
            let economics = birth.economics().ok_or(PublicationError::Missing)?;
            if (row.protocol == EntitlementProtocol::OriginalV3)
                != parent_row.content.parent().is_none()
                || !had_parent_chain
                || economics.grant().royalty_chain() != parent_chain.reference()?
            {
                return Err(PublicationError::Mismatch);
            }
            let rate = economics
                .grant()
                .downstream()
                .royalty_permille()
                .ok_or(PublicationError::Mismatch)?;
            (
                rate,
                RoyaltyBasisV1::ExplicitTermEconomicsV1 {
                    digest: economics.digest()?,
                },
            )
        }
    };
    parent_chain.check_next_resale(royalty_permille)?;
    let edge = RoyaltyEdgeV1 {
        parent,
        charter: selector.charter,
        line: selector.line,
        acquisition: AcquisitionV1 {
            licence: selector.licence,
            term: selector.created_parent_term,
            operation: birth.operation(),
            product_digest: birth.product_digest(),
        },
        royalty_permille,
        basis,
    };
    let mut edges = Vec::with_capacity(1 + parent_chain.edges().len());
    edges.push(edge);
    edges.extend_from_slice(parent_chain.edges());
    capture_or_match(ctx, f, filed, publication, edges, creates)
}

/// Full historical derivation of an already materialized row. The Market
/// publication audit additionally joins its actual immutable listing cause.
pub(crate) fn verify_stored(
    ctx: &impl CommerceRead,
    roll: &RollStore,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
    row: &FiledRoyaltyTermsV1,
) -> Result<(), PublicationError> {
    row.validate()?;
    let prepared = prepare(ctx, roll, f, ratings, row.filed(), row.publication())?;
    if prepared.primary() != row || !prepared.creates().is_empty() {
        return Err(PublicationError::Mismatch);
    }
    Ok(())
}

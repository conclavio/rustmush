//! A structural current-parent selector paired with one real authored Mind.
//! It is never current entitlement. All eligibility is resolved from the
//! selected actual paid grant during each future dependent operation.

use super::filed::ParentContent;
use super::records::{PublicationError, Record};
use super::{CharterId, LicenceId, LineId, TermGeneration};
use crate::{DerivativeProvenance, Mind};
use mush_core::{CommitToken, ObjId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DraftParentSelection {
    pub(crate) format: u16,
    pub(crate) actor: ObjId,
    pub(crate) mind: u64,
    pub(crate) parent: ParentContent,
    pub(crate) charter: CharterId,
    pub(crate) line: LineId,
    pub(crate) licence: LicenceId,
    pub(crate) created_parent_term: TermGeneration,
    pub(crate) generation: u64,
    pub(crate) created: CommitToken,
    pub(crate) operation: CommitToken,
}
impl DraftParentSelection {
    pub(crate) fn key_for(actor: ObjId, mind: u64) -> String {
        format!("draft/{:08x}/{mind:016x}", actor.0)
    }
    pub(crate) fn provenance(&self) -> DerivativeProvenance {
        DerivativeProvenance {
            parent: self.parent,
            charter: self.charter,
            line: self.line,
        }
    }
    pub(crate) fn check_mind(&self, mind: &Mind) -> Result<(), PublicationError> {
        self.validate()?;
        if mind.character != self.actor
            || mind.id != self.mind
            || mind.provenance.as_ref() != Some(&self.provenance())
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
}
impl Record for DraftParentSelection {
    const FAMILY: &'static str = "roll_mind_draft_parent";
    const MAX_BYTES: usize = 2048;
    fn key(&self) -> String {
        Self::key_for(self.actor, self.mind)
    }
    fn validate(&self) -> Result<(), PublicationError> {
        // This first producer creates selectors only. A later reselect/lifecycle
        // protocol must deliberately define and audit generation>1 or retirement.
        if self.format != 1
            || self.mind == 0
            || self.generation != 1
            || self.created != self.operation
        {
            return Err(PublicationError::Invalid);
        }
        Ok(())
    }
}

/// Integrity audit, never a current-rights check. A legitimate own-term lapse,
/// withdrawal or recall does not delete provenance or make durable boot fail.
pub(crate) fn audit(
    ctx: &mush_core::ModuleCtx<'_>,
    roll: &crate::RollStore,
    families: &super::families::CommerceFamilies,
    ratings: &super::rating::RatingFamilies,
    enabled: bool,
) -> Result<(), PublicationError> {
    use super::audit::{read, scan};
    use super::paid::PaidForm;
    use super::records::RevisionPolicy;
    let count = scan(ctx, &families.draft_parents, |selector| {
        if !enabled {
            return Err(PublicationError::Unavailable);
        }
        match ctx
            .action_state(&families.retired_draft_parents, &selector.key())
            .map_err(|_| PublicationError::Unavailable)?
        {
            Some(tombstone) => {
                super::authored_retirement::audit::check(ctx, roll, families, selector, tombstone)?
            }
            None => selector.check_mind(
                roll.owned_mind(selector.actor, selector.mind)
                    .map_err(|_| PublicationError::Mismatch)?,
            )?,
        }
        let filed = read(ctx, &families.filed, selector.parent.filed.key())?;
        if filed.content.parent().is_some()
            || filed.content.revision() != selector.parent.revision
            || filed.content.position().charter != selector.charter
            || filed.content.position().line != selector.line
        {
            return Err(PublicationError::Mismatch);
        }
        let licence = read(ctx, &families.paid_licences, selector.licence.key())?;
        let term = read(
            ctx,
            &families.paid_terms,
            format!(
                "{}/{}",
                selector.licence.key(),
                selector.created_parent_term.key()
            ),
        )?;
        let allocation =
            super::entitlement::history::granted(ctx, families, ratings, licence, term)?;
        // These historical rows have already passed the full paid/E4 audit.
        // Do not consult the parent's seller or today's eligibility here.
        if allocation.holder() != selector.actor
            || allocation.licence() != selector.licence
            || allocation.term() != selector.created_parent_term
            || licence.charter() != selector.charter
            || licence.line() != selector.line
            || term.licence() != selector.licence
            || term.generation() != selector.created_parent_term
            || !matches!(
                term.terms().form(),
                PaidForm::Open {} | PaidForm::OpenResale {}
            )
            || term.policy()
                != (RevisionPolicy::Pin {
                    filed: selector.parent.filed,
                    revision: selector.parent.revision,
                })
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    })?;
    // Mandatory even at zero typed rows: missing-all selectors never means a
    // healthy empty pairing. No global licence scan or duplicated rights index.
    let mut minds = 0_u64;
    for mind in roll.derivative_minds() {
        minds = minds.checked_add(1).ok_or(PublicationError::Capacity)?;
        if !enabled || minds > crate::MAX_RECORDS as u64 {
            return Err(PublicationError::Unavailable);
        }
        read(
            ctx,
            &families.draft_parents,
            DraftParentSelection::key_for(mind.character, mind.id),
        )?
        .check_mind(mind)?;
    }
    let retired = super::authored_retirement::audit::audit(ctx, roll, families, enabled)?;
    if minds
        .checked_add(retired)
        .ok_or(PublicationError::Capacity)?
        != count
    {
        return Err(PublicationError::Mismatch);
    }
    Ok(())
}

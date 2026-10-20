//! Accepted qualification for a compatible derived revision. The current
//! provider captures it; historical reads never substitute today's head.
use super::records::UpdateOperation;
use crate::commerce::{
    CharterId, LineId, TermPhase, WorldIcSeconds,
    dependent_owner::OwnerParentBinding,
    entitlement::history as paid,
    families::CommerceFamilies,
    filed::ParentContent,
    owner_qualification::records::{QualificationOperation, SelectedParent},
    rating::RatingFamilies,
    read::CommerceRead,
    records::{InitialFilingOperation, OwnerTerm, PublicationError, Record, RevisionPolicy},
};
use mush_core::CommitToken;
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "ParentWire")]
pub(crate) struct DerivedParent {
    format: u16,
    pub(crate) source: ParentContent,
    pub(crate) charter: CharterId,
    pub(crate) line: LineId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) qualification: Option<CommitToken>,
    pub(crate) selected: SelectedParent,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ParentWire {
    format: u16,
    source: ParentContent,
    charter: CharterId,
    line: LineId,
    #[serde(default, deserialize_with = "present_commit")]
    qualification: Option<CommitToken>,
    selected: SelectedParent,
}
fn present_commit<'de, D: Deserializer<'de>>(d: D) -> Result<Option<CommitToken>, D::Error> {
    CommitToken::deserialize(d).map(Some)
}
pub(super) fn present_parent<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<DerivedParent>, D::Error> {
    DerivedParent::deserialize(d).map(Some)
}
impl TryFrom<ParentWire> for DerivedParent {
    type Error = PublicationError;
    fn try_from(w: ParentWire) -> Result<Self, Self::Error> {
        let value = Self {
            format: w.format,
            source: w.source,
            charter: w.charter,
            line: w.line,
            qualification: w.qualification,
            selected: w.selected,
        };
        value.validate()?;
        Ok(value)
    }
}
impl DerivedParent {
    pub(super) fn capture(
        binding: &OwnerParentBinding,
        qualification: Option<CommitToken>,
        actual: &crate::commerce::parent_grant::CurrentParentGrant,
    ) -> Result<Self, PublicationError> {
        if actual.holding().charter() != binding.parent.charter
            || actual.holding().line() != binding.parent.line
        {
            return Err(PublicationError::Mismatch);
        }
        let value = Self {
            format: 1,
            source: binding.parent.parent,
            charter: binding.parent.charter,
            line: binding.parent.line,
            qualification,
            selected: SelectedParent::capture(actual.holding()),
        };
        value.validate()?;
        Ok(value)
    }
    pub(super) fn validate(&self) -> Result<(), PublicationError> {
        self.selected.validate()?;
        if self.format != 1 {
            return Err(PublicationError::Invalid);
        }
        Ok(())
    }
    pub(super) fn matches_birth(
        &self,
        op: &UpdateOperation,
        birth: &InitialFilingOperation,
    ) -> Result<(), PublicationError> {
        self.validate()?;
        let initial = birth.parent.as_ref().ok_or(PublicationError::Mismatch)?;
        let at = WorldIcSeconds::from_stamp(&op.at_ic).map_err(|_| PublicationError::Invalid)?;
        if self.source != initial.parent
            || self.charter != initial.charter
            || self.line != initial.line
            || self.line == op.line
            || !matches!(
                self.selected.window.phase_at(at),
                TermPhase::Active | TermPhase::Grace
            )
            || (self.qualification.is_none()
                && (op.charter_ownership.is_some()
                    || self.selected.licence != initial.licence
                    || self.selected.filed != initial.parent.filed
                    || self.selected.revision != initial.parent.revision))
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
    pub(super) fn matches_qualification(
        &self,
        op: &UpdateOperation,
        qualification: &QualificationOperation,
    ) -> Result<(), PublicationError> {
        qualification.validate()?;
        if self.qualification != Some(qualification.operation)
            || qualification.owner != op.actor
            || qualification.owner_licence != op.licence
            || qualification.birth_operation != op.initial
            || qualification.charter != op.charter
            || qualification.line != op.line
            || qualification.source_parent != self.source
            || qualification.parent_charter != self.charter
            || qualification.parent_line != self.line
            || qualification.selected_parent.licence != self.selected.licence
            || qualification.charter_ownership != op.charter_ownership
            || (qualification.observed_ic.clock == op.at_ic.clock
                && qualification.observed_ic.at > op.at_ic.at)
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
}

/// Read the actual current qualification once through the existing registered
/// ParentGrant service. Every mutable selection/status row is a transaction
/// preimage; the opaque grant retains its own owner/readiness observations.
pub(super) fn prepare_parent(
    txn: &mut mush_core::ActionTxn<'_>,
    ctx: &mut mush_core::PrepareCtx<'_>,
    families: &CommerceFamilies,
    reader: &mush_core::read_service::OwnerReadHandle<
        crate::commerce::parent_grant::ParentGrantReader,
    >,
    birth: &InitialFilingOperation,
    ownership: &crate::commerce::charter_transfer::records::CharterOwnershipRef,
    actor: mush_core::ObjId,
) -> Result<
    (
        DerivedParent,
        crate::commerce::parent_grant::CurrentParentGrant,
    ),
    PublicationError,
> {
    use crate::commerce::owner_qualification::records::QualificationHead;
    let invalid = |_| PublicationError::Unavailable;
    let binding = txn
        .get(
            &families.owner_parents,
            &OwnerParentBinding::key_for(birth.licence, birth.requester),
        )
        .map_err(invalid)?
        .cloned()
        .ok_or(PublicationError::Missing)?;
    let filed = txn
        .get(&families.filed, &birth.filed.key())
        .map_err(invalid)?
        .cloned()
        .ok_or(PublicationError::Missing)?;
    let term = txn
        .get(
            &families.terms,
            &OwnerTerm::encoded_key(birth.licence, birth.term),
        )
        .map_err(invalid)?
        .cloned()
        .ok_or(PublicationError::Missing)?;
    binding.check_rows(&filed, &term, birth)?;
    let source = txn
        .get(&families.filed, &binding.parent.parent.filed.key())
        .map_err(invalid)?
        .cloned()
        .ok_or(PublicationError::Missing)?;
    let status = txn
        .get(&families.status, &binding.parent.parent.filed.key())
        .map_err(invalid)?
        .cloned()
        .ok_or(PublicationError::Missing)?;
    source.validate()?;
    status.validate()?;
    if source.content.id() != binding.parent.parent.filed
        || source.content.revision() != binding.parent.parent.revision
        || source.content.position().charter != binding.parent.charter
        || source.content.position().line != binding.parent.line
        || status.filed != binding.parent.parent.filed
        || status.recalled_by.is_some()
    {
        return Err(PublicationError::Unavailable);
    }
    let head = txn
        .get(
            &families.owner_qualification_heads,
            &QualificationHead::key_for(birth.licence, actor),
        )
        .map_err(invalid)?
        .cloned();
    let (selected, cause) = match head {
        None if ownership.generation == 1 => (binding.parent.licence, None),
        None => return Err(PublicationError::Unavailable),
        Some(head) => {
            let q = txn
                .get(
                    &families.owner_qualification_operations,
                    &head.operation.to_hex(),
                )
                .map_err(invalid)?
                .cloned()
                .ok_or(PublicationError::Missing)?;
            head.matches(&q)?;
            q.matches_birth(&binding, &filed, &term, birth)?;
            if q.charter_ownership.as_ref() != (ownership.generation > 1).then_some(ownership) {
                return Err(PublicationError::Unavailable);
            }
            crate::commerce::charter_transfer::history::require_captured_in_txn(
                txn,
                ctx.world().map_err(|_| PublicationError::Unavailable)?,
                families,
                birth,
                q.charter_ownership.as_ref(),
                actor,
            )?;
            ctx.world()
                .map_err(|_| PublicationError::Unavailable)?
                .module_state::<crate::RollStore>()
                .ok_or(PublicationError::Unavailable)?
                .check_required_root(&q.root()?)
                .map_err(|_| PublicationError::Mismatch)?;
            (q.selected_parent.licence, Some(q.operation))
        }
    };
    let actual = ctx
        .read_owner_preimage(
            txn,
            reader,
            crate::commerce::parent_grant::ParentGrantRequest { licence: selected },
        )
        .map_err(|_| PublicationError::Unavailable)?;
    actual.check_fresh(
        actor,
        ctx.world().map_err(|_| PublicationError::Unavailable)?,
    )?;
    let evidence = DerivedParent::capture(&binding, cause, &actual)?;
    let roll = ctx
        .world()
        .map_err(|_| PublicationError::Unavailable)?
        .module_state::<crate::RollStore>()
        .ok_or(PublicationError::Unavailable)?;
    roll.check_required_root(&crate::commerce::roots::RequiredRoot {
        anchor: crate::commerce::roots::RootAnchor::Filed(binding.parent.parent.filed),
        identity: binding.parent.parent.revision,
    })
    .map_err(|_| PublicationError::Mismatch)?;
    Ok((evidence, actual))
}

/// Full permanent proof, used by the complete boot audit. No current source,
/// parent phase, title, recall or certificate-standing predicate occurs here.
pub(super) fn historical(
    ctx: &impl CommerceRead,
    roll: &crate::RollStore,
    families: &CommerceFamilies,
    ratings: &RatingFamilies,
    op: &UpdateOperation,
    birth: &InitialFilingOperation,
) -> Result<(), PublicationError> {
    let parent = op.parent.as_ref().ok_or(PublicationError::Mismatch)?;
    parent.matches_birth(op, birth)?;
    let binding = paid::read(
        ctx,
        &families.owner_parents,
        OwnerParentBinding::key_for(birth.licence, birth.requester),
    )?;
    let initial_filed = paid::read(ctx, &families.filed, birth.filed.key())?;
    let initial_term = paid::read(
        ctx,
        &families.terms,
        OwnerTerm::encoded_key(birth.licence, birth.term),
    )?;
    crate::commerce::dependent_owner::verify_history(
        ctx,
        roll,
        families,
        ratings,
        binding,
        initial_filed,
        initial_term,
        birth,
    )?;
    if let Some(commit) = parent.qualification {
        let qualification = paid::read(
            ctx,
            &families.owner_qualification_operations,
            commit.to_hex(),
        )?;
        parent.matches_qualification(op, qualification)?;
        crate::commerce::owner_qualification::history::verify(
            ctx,
            roll,
            families,
            ratings,
            qualification,
        )?;
    }
    let selected = &parent.selected;
    let licence = paid::read(ctx, &families.paid_licences, selected.licence.key())?;
    let term = paid::read(
        ctx,
        &families.paid_terms,
        format!("{}/{}", selected.licence.key(), selected.term.key()),
    )?;
    let grant = paid::granted(ctx, families, ratings, licence, term)?;
    crate::commerce::owner_qualification::history::require_selected_holder(
        ctx,
        families,
        ratings,
        licence,
        selected.holder_basis.as_ref(),
        grant.holder(),
        op.actor,
        &op.at_ic,
    )?;
    if grant.operation() != selected.operation
        || grant.terminal() != selected.terminal
        || grant.licence() != selected.licence
        || grant.term() != selected.term
        || grant.charter() != parent.charter
        || grant.line() != parent.line
        || grant.filed() != selected.filed
        || grant.revision() != selected.revision
        || term.window() != selected.window
        || term.terms().form() != selected.form
        || term.policy()
            != (RevisionPolicy::Pin {
                filed: selected.filed,
                revision: selected.revision,
            })
    {
        return Err(PublicationError::Mismatch);
    }
    let revision = roll
        .revision(
            op.revision.revision().key.character,
            op.revision.revision().key,
        )
        .map_err(|_| PublicationError::Unavailable)?;
    if revision.provenance.as_ref()
        != Some(&crate::DerivativeProvenance {
            parent: parent.source,
            charter: parent.charter,
            line: parent.line,
        })
    {
        return Err(PublicationError::Mismatch);
    }
    Ok(())
}

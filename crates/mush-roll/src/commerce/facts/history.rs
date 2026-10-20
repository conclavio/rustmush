//! Exact accepted generations; no current term, Registry or parent eligibility.
use super::super::{
    LicenceId, TermGeneration, WorldIcSeconds,
    entitlement::history::{granted, optional, read},
    families::CommerceFamilies,
    paid::{PaidTermRecord, PersonalPaidLicence},
    rating::RatingFamilies,
    read::CommerceRead,
    records::{InitialFilingOperation, OwnerLicence, OwnerTerm, PublicationError, Record},
    roots::{RequiredRoot, RootAnchor},
};
use super::records::{AcceptedTerm, PersonalKind, term_digest};
use crate::RollStore;
use mush_core::{ActionTxn, ClockStamp, StateFamilyHandle};
use serde::Serialize;

fn unavailable<T>(_: T) -> PublicationError {
    PublicationError::Unavailable
}
fn mismatch<T>(_: T) -> PublicationError {
    PublicationError::Mismatch
}

/// A bounded observation minted from actual owner reads. It cannot be decoded
/// from parameters or reused as source/admission authority.
#[derive(Debug, Serialize)]
pub(super) struct TermProof {
    pub(super) accepted: AcceptedTerm,
    rows: Rows,
}
#[derive(Debug, Serialize)]
enum Rows {
    Owner {
        licence: OwnerLicence,
        term: OwnerTerm,
        birth: InitialFilingOperation,
        successor: Option<OwnerTerm>,
    },
    Paid {
        licence: PersonalPaidLicence,
        term: PaidTermRecord,
        successor: Option<PaidTermRecord>,
    },
}
impl TermProof {
    /// A normal renewal before the old grace boundary prevents that generation
    /// from lapsing. A replacement at the boundary or later preserves the gap.
    pub(super) fn require_lapsed(&self, now: &ClockStamp) -> Result<(), PublicationError> {
        self.accepted.expired(now)?;
        let successor = match &self.rows {
            Rows::Owner { successor, .. } => successor.as_ref().map(|t| t.window.issued_at()),
            Rows::Paid { successor, .. } => successor.as_ref().map(|t| t.window().issued_at()),
        };
        require_expired_generation(self.accepted.window.grace_end(), successor)
    }
    pub(super) fn observed_head(&self) -> TermGeneration {
        match &self.rows {
            Rows::Owner { licence, .. } => licence.term,
            Rows::Paid { licence, .. } => licence.term(),
        }
    }
    pub(super) fn require_historical_lapse(
        &self,
        row: &super::records::LapseOccurrence,
    ) -> Result<(), PublicationError> {
        row.matches(&self.accepted)?;
        if row.observed_head > self.observed_head() {
            return Err(PublicationError::Mismatch);
        }
        if row.observed_head > row.term.generation {
            self.require_lapsed(&row.at_ic)
        } else {
            row.term.expired(&row.at_ic)
        }
    }
    pub(super) fn paid_current_holder(&self) -> Option<mush_core::ObjId> {
        match &self.rows {
            Rows::Paid { licence, .. } => Some(licence.holder()),
            Rows::Owner { .. } => None,
        }
    }
    /// Admit only while the exact current pointer and historical rows remain
    /// the preimage observed by the actual owner reader. Root generation covers
    /// retained root changes; these typed reads also close successor insertion.
    pub(super) fn require_before(
        &self,
        txn: &mut ActionTxn<'_>,
        f: &CommerceFamilies,
    ) -> Result<(), PublicationError> {
        match &self.rows {
            Rows::Owner {
                licence,
                term,
                birth,
                successor,
            } => {
                before(txn, &f.licences, licence)?;
                before(txn, &f.terms, term)?;
                before(txn, &f.operations, birth)?;
                successor_before(
                    txn,
                    &f.terms,
                    licence.id,
                    term.generation,
                    successor.as_ref(),
                )?;
                absent(txn, &f.paid_licences, &licence.id.key())?;
            }
            Rows::Paid {
                licence,
                term,
                successor,
            } => {
                before(txn, &f.paid_licences, licence)?;
                before(txn, &f.paid_terms, term)?;
                successor_before(
                    txn,
                    &f.paid_terms,
                    licence.id(),
                    term.generation(),
                    successor.as_ref(),
                )?;
                absent(txn, &f.licences, &licence.id().key())?;
            }
        }
        Ok(())
    }
}
fn require_expired_generation(
    grace_end: WorldIcSeconds,
    successor_issued: Option<WorldIcSeconds>,
) -> Result<(), PublicationError> {
    if successor_issued.is_some_and(|at| at < grace_end) {
        return Err(PublicationError::Unavailable);
    }
    Ok(())
}
fn before<R: Record + PartialEq>(
    txn: &mut ActionTxn<'_>,
    family: &StateFamilyHandle<String, R>,
    row: &R,
) -> Result<(), PublicationError> {
    if txn.get(family, &row.key()).map_err(unavailable)? != Some(row) {
        return Err(PublicationError::Mismatch);
    }
    Ok(())
}
fn absent<R: Record>(
    txn: &mut ActionTxn<'_>,
    family: &StateFamilyHandle<String, R>,
    key: &String,
) -> Result<(), PublicationError> {
    if txn.get(family, key).map_err(unavailable)?.is_some() {
        return Err(PublicationError::Mismatch);
    }
    Ok(())
}
fn successor_before<R: Record + PartialEq>(
    txn: &mut ActionTxn<'_>,
    family: &StateFamilyHandle<String, R>,
    licence: LicenceId,
    generation: TermGeneration,
    row: Option<&R>,
) -> Result<(), PublicationError> {
    if let Some(next) = next_generation(generation)? {
        let key = format!("{}/{}", licence.key(), next.key());
        if txn.get(family, &key).map_err(unavailable)? != row {
            return Err(PublicationError::Mismatch);
        }
    } else if row.is_some() {
        return Err(PublicationError::Mismatch);
    }
    Ok(())
}
fn next_generation(g: TermGeneration) -> Result<Option<TermGeneration>, PublicationError> {
    g.get()
        .checked_add(1)
        .map(TermGeneration::new)
        .transpose()
        .map_err(mismatch)
}

pub(super) fn resolve(
    ctx: &impl CommerceRead,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
    licence: LicenceId,
    generation: TermGeneration,
) -> Result<TermProof, PublicationError> {
    let roll = ctx
        .world()
        .module_state::<RollStore>()
        .ok_or(PublicationError::Unavailable)?;
    roll.commerce_ready().map_err(unavailable)?;
    historical(ctx, roll, f, ratings, licence, generation)
}
/// Boot calls this only inside the mandatory full family/root audit, while
/// commerce is intentionally closed. Live entry calls resolve() above.
pub(super) fn historical(
    ctx: &impl CommerceRead,
    roll: &RollStore,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
    licence: LicenceId,
    generation: TermGeneration,
) -> Result<TermProof, PublicationError> {
    let owner = optional(ctx, &f.licences, licence.key())?;
    let paid = optional(ctx, &f.paid_licences, licence.key())?;
    match (owner, paid) {
        (Some(owner), None) => owner_term(ctx, roll, f, owner, generation),
        (None, Some(paid)) => paid_term(ctx, roll, f, ratings, paid, generation),
        _ => Err(PublicationError::Unavailable),
    }
}
fn owner_term(
    ctx: &impl CommerceRead,
    roll: &RollStore,
    f: &CommerceFamilies,
    licence: &OwnerLicence,
    generation: TermGeneration,
) -> Result<TermProof, PublicationError> {
    if generation > licence.term {
        return Err(PublicationError::Unavailable);
    }
    let birth = read(ctx, &f.operations, licence.operation.to_hex())?;
    let mush_org::PrincipalId::Object(initial_owner) = birth.economic.issued_owner else {
        return Err(PublicationError::Unavailable);
    };
    if birth.economic.house.is_some()
        || birth.licence != licence.id
        || birth.charter != licence.charter
        || birth.line != licence.line
        || birth.requester != initial_owner
    {
        return Err(PublicationError::Mismatch);
    }
    let economy = ctx
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .ok_or(PublicationError::Unavailable)?;
    if economy.is_frozen() {
        return Err(PublicationError::Unavailable);
    }
    economy
        .filing_receipt_audit()
        .verify(&birth.audit_subject()?)
        .map_err(unavailable)?;
    roll.check_required_root(&RequiredRoot {
        anchor: RootAnchor::Operation(birth.commit),
        identity: birth.revision,
    })
    .map_err(mismatch)?;
    let term =
        super::super::owner_attestation::history::historical(ctx, roll, f, birth, generation)?;
    let (accepted_holder, revision) = if generation.get() == 1 {
        (initial_owner, birth.revision)
    } else {
        let op = read(ctx, &f.owner_attestations, term.operation.to_hex())?;
        // historical() already checked exact cause, predecessor and its roots.
        (op.owner, op.revision)
    };
    let successor = if generation < licence.term {
        let next = next_generation(generation)?.ok_or(PublicationError::Mismatch)?;
        Some(super::super::owner_attestation::history::historical(
            ctx, roll, f, birth, next,
        )?)
    } else {
        if let Some(next) = next_generation(generation)? {
            if optional(ctx, &f.terms, OwnerTerm::encoded_key(licence.id, next))?.is_some() {
                return Err(PublicationError::Mismatch);
            }
        }
        None
    };
    let accepted = AcceptedTerm {
        kind: PersonalKind::Owner,
        licence: licence.id,
        generation,
        birth: birth.commit,
        term_operation: term.operation,
        term_digest: term_digest(PersonalKind::Owner, &term)?,
        accepted_holder,
        charter: licence.charter,
        line: licence.line,
        revision,
        window: term.window,
    };
    accepted.validate()?;
    Ok(TermProof {
        accepted,
        rows: Rows::Owner {
            licence: licence.clone(),
            term,
            birth: birth.clone(),
            successor,
        },
    })
}
fn paid_term(
    ctx: &impl CommerceRead,
    roll: &RollStore,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
    licence: &PersonalPaidLicence,
    generation: TermGeneration,
) -> Result<TermProof, PublicationError> {
    if generation > licence.term() {
        return Err(PublicationError::Unavailable);
    }
    let key = format!("{}/{}", licence.id().key(), generation.key());
    let term = read(ctx, &f.paid_terms, key)?;
    let actual = granted(ctx, f, ratings, licence, term)?;
    // Current scope follows the actual complete holder chain, independently of
    // the person who accepted this historical term generation.
    super::super::paid_resale::history::current(ctx, f, ratings, licence)?;
    let revision = actual.revision();
    roll.check_required_root(&RequiredRoot {
        anchor: RootAnchor::Term(licence.id(), generation),
        identity: revision,
    })
    .map_err(mismatch)?;
    let successor = if generation < licence.term() {
        let next = next_generation(generation)?.ok_or(PublicationError::Mismatch)?;
        let row = read(
            ctx,
            &f.paid_terms,
            format!("{}/{}", licence.id().key(), next.key()),
        )?;
        granted(ctx, f, ratings, licence, row)?;
        Some(row.clone())
    } else {
        if let Some(next) = next_generation(generation)? {
            if optional(
                ctx,
                &f.paid_terms,
                format!("{}/{}", licence.id().key(), next.key()),
            )?
            .is_some()
            {
                return Err(PublicationError::Mismatch);
            }
        }
        None
    };
    let accepted = AcceptedTerm {
        kind: PersonalKind::Paid,
        licence: licence.id(),
        generation,
        birth: actual.birth_operation(),
        term_operation: actual.operation(),
        term_digest: term_digest(PersonalKind::Paid, term)?,
        accepted_holder: actual.holder(),
        charter: actual.charter(),
        line: actual.line(),
        revision,
        window: term.window(),
    };
    accepted.validate()?;
    Ok(TermProof {
        accepted,
        rows: Rows::Paid {
            licence: licence.clone(),
            term: term.clone(),
            successor,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn successor_before_expiry_prevents_lapse_but_exact_or_later_does_not_erase_it() {
        let boundary = WorldIcSeconds(100);
        assert!(require_expired_generation(boundary, None).is_ok());
        assert!(require_expired_generation(boundary, Some(WorldIcSeconds(99))).is_err());
        assert!(require_expired_generation(boundary, Some(WorldIcSeconds(100))).is_ok());
        assert!(require_expired_generation(boundary, Some(WorldIcSeconds(101))).is_ok());
        assert!(
            next_generation(TermGeneration::new(u64::MAX).unwrap())
                .unwrap()
                .is_none()
        );
    }
}

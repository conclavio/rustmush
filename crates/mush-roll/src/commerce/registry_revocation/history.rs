//! Exact historical issuance proof. No current parent/source/Registry status
//! is consulted while auditing an already committed sanction.
use super::super::{
    entitlement::history::read, families::CommerceFamilies, rating::RatingFamilies,
    read::CommerceRead, records::OwnerTerm,
};
use super::*;
use records::{AcceptedTerm, RevocationOperation};
pub(crate) fn verify(
    ctx: &impl CommerceRead,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
    op: &RevocationOperation,
) -> Result<(), PublicationError> {
    op.validate()?;
    let roll = ctx
        .world()
        .module_state::<crate::RollStore>()
        .ok_or(PublicationError::Unavailable)?;
    let filed = read(ctx, &f.filed, op.filed.key())?;
    let initial = super::super::line_update::audit::birth(ctx, f, filed)?;
    if initial.commit != op.initial_operation
        || initial.economic.issuer != op.initial_issuer
        || filed.content.revision() != op.revision
        || filed.content.position().charter != op.charter
        || filed.content.position().line != op.line
        || initial.charter != op.charter
        || initial.line != op.line
    {
        return Err(PublicationError::Mismatch);
    }
    match &op.term {
        AcceptedTerm::Owner { term } => {
            let licence = read(ctx, &f.licences, term.licence.key())?;
            if ctx
                .get(&f.paid_licences, &term.licence.key())
                .map_err(bad)?
                .is_some()
                || licence.operation != op.birth_operation
                || licence.operation != initial.commit
                || licence.charter != op.charter
                || licence.line != op.line
                || &super::super::owner_attestation::history::historical(
                    ctx,
                    roll,
                    f,
                    initial,
                    term.generation,
                )? != term.as_ref()
            {
                return Err(PublicationError::Mismatch);
            }
        }
        AcceptedTerm::Paid { term } => {
            let licence = read(ctx, &f.paid_licences, term.licence().key())?;
            if ctx
                .get(&f.licences, &term.licence().key())
                .map_err(bad)?
                .is_some()
                || licence.operation() != op.birth_operation
                || licence.charter() != op.charter
                || licence.line() != op.line
                || read(ctx, &f.paid_terms, term.key())? != term.as_ref()
            {
                return Err(PublicationError::Mismatch);
            }
            super::super::entitlement::history::granted(ctx, f, ratings, licence, term)?;
        }
    }
    roll.check_required_root(&RequiredRoot {
        anchor: RootAnchor::Filed(op.filed),
        identity: op.revision,
    })
    .map_err(bad)?;
    roll.check_required_root(&op.root()?).map_err(bad)
}
/// Real current accepted rows in the actual transaction. Full cold audits prove
/// the historical family; the native E4 journals are checked again below.
pub(crate) fn capture(
    txn: &mut mush_core::ActionTxn<'_>,
    world: &mush_core::World,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
    id: LicenceId,
) -> Result<
    (
        AcceptedTerm,
        super::super::CharterId,
        super::super::LineId,
        CommitToken,
    ),
    PublicationError,
> {
    let owner = txn.get(&f.licences, &id.key()).map_err(bad)?.cloned();
    let paid = txn.get(&f.paid_licences, &id.key()).map_err(bad)?.cloned();
    match (owner, paid) {
        (Some(owner), None) => {
            if owner.id != id {
                return Err(PublicationError::Mismatch);
            }
            let term =
                super::super::owner_attestation::history::current_in_txn(txn, world, f, &owner)?;
            Ok((
                AcceptedTerm::Owner {
                    term: Box::new(term),
                },
                owner.charter,
                owner.line,
                owner.operation,
            ))
        }
        (None, Some(paid)) => {
            if paid.id() != id {
                return Err(PublicationError::Mismatch);
            }
            let term = txn
                .get(&f.paid_terms, &OwnerTerm::encoded_key(id, paid.term()))
                .map_err(bad)?
                .cloned()
                .ok_or(PublicationError::Missing)?;
            term.matches_current(&paid)?;
            paid_current(txn, world, f, ratings, &paid, &term)?;
            Ok((
                AcceptedTerm::Paid {
                    term: Box::new(term),
                },
                paid.charter(),
                paid.line(),
                paid.operation(),
            ))
        }
        _ => Err(PublicationError::Missing),
    }
}
fn paid_current(
    txn: &mut mush_core::ActionTxn<'_>,
    world: &mush_core::World,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
    licence: &super::super::paid::PersonalPaidLicence,
    term: &super::super::paid::PaidTermRecord,
) -> Result<(), PublicationError> {
    use super::super::entitlement::history as h;
    let economy = world
        .module_state::<mush_economy::EconomyStore>()
        .ok_or(PublicationError::Unavailable)?;
    if economy.is_frozen() {
        return Err(PublicationError::Unavailable);
    }
    let audit = economy.purchase_receipt_audit();
    audit.require_complete().map_err(bad)?;
    let actual = audit.get(licence.operation()).map_err(bad)?;
    let old = txn
        .get(&f.paid_allocations, &licence.operation().to_hex())
        .map_err(bad)?
        .cloned();
    let new = txn
        .get(&f.entitlement_allocations, &licence.operation().to_hex())
        .map_err(bad)?
        .cloned();
    let house = txn
        .get(&f.house_allocations, &licence.operation().to_hex())
        .map_err(bad)?
        .cloned();
    let (rating, birth_generation, revision, class) = match (&old, &new, &house) {
        (Some(a), None, None) => (a.rating(), a.term(), a.revision(), a.terms().class()),
        (None, Some(a), None) => (a.rating(), a.term(), a.revision(), a.terms().class()),
        (None, None, Some(a)) => (a.rating(), a.term(), a.revision(), a.terms().class()),
        _ => return Err(PublicationError::Mismatch),
    };
    let receipt = txn
        .get(&ratings.receipts, &rating.job())
        .map_err(bad)?
        .cloned()
        .ok_or(PublicationError::Missing)?;
    receipt.validate().map_err(bad)?;
    if receipt.reference().map_err(bad)? != rating
        || receipt.subject().revision() != revision
        || class < receipt.minimum()
        || term.licence() != licence.id()
    {
        return Err(PublicationError::Mismatch);
    }
    let birth_term = txn
        .get(
            &f.paid_terms,
            &OwnerTerm::encoded_key(licence.id(), birth_generation),
        )
        .map_err(bad)?
        .cloned()
        .ok_or(PublicationError::Missing)?;
    let birth_economics = txn
        .get(&f.term_economics, &birth_term.key())
        .map_err(bad)?
        .cloned();
    let house_economics = txn
        .get(&f.house_economics, &birth_term.key())
        .map_err(bad)?
        .cloned();
    if (house.is_some() && birth_economics.is_some())
        || (house.is_none() && house_economics.is_some())
    {
        return Err(PublicationError::Mismatch);
    }
    let birth = match (&old, &new, &house) {
        (Some(a), None, None) => h::Birth::Legacy(a),
        (None, Some(a), None) => h::Birth::Entitlement(a),
        (None, None, Some(a)) => h::Birth::House(a),
        _ => return Err(PublicationError::Mismatch),
    };
    let birth_pointer = super::super::paid::PersonalPaidLicence::original(
        birth.licence(),
        birth.charter(),
        birth.line(),
        birth.holder(),
        licence.term(),
        birth.operation(),
    )?;
    super::super::paid_resale::history::current_in_txn(txn, world, f, licence, birth)?;
    match (&old, &new, &house) {
        (Some(row), None, None) => {
            if birth_economics.is_some() || actual.entitlement().map_err(bad)?.is_some() {
                return Err(PublicationError::Mismatch);
            }
            h::verify_legacy_history(row, actual, receipt.compatibility().digest())?;
            super::super::paid_audit::verify_granted(row, &birth_pointer, &birth_term, actual)?;
        }
        (None, Some(row), None) => h::verify_entitlement_grant(
            row,
            &birth_pointer,
            &birth_term,
            birth_economics.as_ref().ok_or(PublicationError::Missing)?,
            actual,
            receipt.compatibility().digest(),
        )?,
        (None, None, Some(row)) => super::super::house_purchase::history::verify_grant(
            row,
            &birth_pointer,
            &birth_term,
            house_economics.as_ref().ok_or(PublicationError::Missing)?,
            actual,
            receipt.compatibility().digest(),
        )?,
        _ => return Err(PublicationError::Mismatch),
    }
    let proof = CurrentTerm {
        txn,
        f,
        economy,
        licence,
        term,
        birth_generation,
        birth_term: &birth_term,
    };
    if house.is_some() {
        proof.house(&house)?;
    } else {
        proof.personal(&new)?;
    }
    Ok(())
}

struct CurrentTerm<'a, 'w> {
    txn: &'a mut mush_core::ActionTxn<'w>,
    f: &'a CommerceFamilies,
    economy: &'a mush_economy::EconomyStore,
    licence: &'a super::super::paid::PersonalPaidLicence,
    term: &'a super::super::paid::PaidTermRecord,
    birth_generation: super::super::TermGeneration,
    birth_term: &'a super::super::paid::PaidTermRecord,
}
impl CurrentTerm<'_, '_> {
    #[inline(never)]
    fn house(
        self,
        house: &Option<super::super::house_purchase::HouseAllocationV1>,
    ) -> Result<(), PublicationError> {
        let Self {
            txn,
            f,
            economy,
            licence,
            term,
            birth_generation,
            birth_term,
        } = self;
        if term.generation() == birth_generation {
            if term != birth_term {
                return Err(PublicationError::Mismatch);
            }
        } else {
            let row = txn
                .get(&f.house_renewal_operations, &term.operation().to_hex())
                .map_err(bad)?
                .cloned()
                .ok_or(PublicationError::Missing)?;
            let economy_row = txn
                .get(&f.house_economics, &term.key())
                .map_err(bad)?
                .cloned()
                .ok_or(PublicationError::Missing)?;
            let view = economy
                .house_renewal_audit()
                .map_err(bad)?
                .get(row.operation())
                .map_err(bad)?;
            super::super::house_renewal::history::verify(&row, &view)?;
            let (_, expected_term, expected_economics) = row.granted_rows()?;
            let Some(birth) = house.as_ref() else {
                return Err(PublicationError::Mismatch);
            };
            let accepted_pointer = row.binding().accepted_pointer(licence.term())?;
            if !row.binding().matches_birth(&accepted_pointer)
                || term != &expected_term
                || economy_row != expected_economics
                || row.binding().issuer() != birth.issuer()
                || row.binding().product_digest() != birth.product_digest()
                || row.binding().grant() != birth.economics().grant()
                || view.offer().basis.prior_quote != birth.quote()
                || view.offer().basis.prior_quote_digest != birth.quote_digest()
            {
                return Err(PublicationError::Mismatch);
            }
            let prior = txn
                .get(
                    &f.paid_terms,
                    &OwnerTerm::encoded_key(licence.id(), row.binding().predecessor()),
                )
                .map_err(bad)?
                .cloned()
                .ok_or(PublicationError::Missing)?;
            let prior_economics = txn
                .get(&f.house_economics, &prior.key())
                .map_err(bad)?
                .cloned()
                .ok_or(PublicationError::Missing)?;
            row.binding()
                .check_predecessor(&accepted_pointer, &prior, &prior_economics)?;
        }
        let head = txn
            .get(&f.house_renewal_heads, &licence.id().key())
            .map_err(bad)?
            .cloned();
        match head {
            None if licence.term().get() == 1 => {}
            Some(head) => {
                let row = txn
                    .get(&f.house_renewal_operations, &head.last_operation().to_hex())
                    .map_err(bad)?
                    .cloned()
                    .ok_or(PublicationError::Missing)?;
                if head != super::super::house_renewal::RenewalHeadV1::for_operation(&row)?
                    || head.current() != licence.term()
                    || !row
                        .binding()
                        .matches_birth(&row.binding().accepted_pointer(licence.term())?)
                    || (row.state().pending() && !row.binding().matches_birth(licence))
                {
                    return Err(PublicationError::Mismatch);
                }
                let view = economy
                    .house_renewal_audit()
                    .map_err(bad)?
                    .get(row.operation())
                    .map_err(bad)?;
                super::super::house_renewal::history::verify(&row, &view)?;
            }
            _ => return Err(PublicationError::Mismatch),
        }
        Ok(())
    }
    #[inline(never)]
    fn personal(
        self,
        new: &Option<super::super::entitlement::EntitlementAllocationV1>,
    ) -> Result<(), PublicationError> {
        let Self {
            txn,
            f,
            economy,
            licence,
            term,
            birth_generation,
            birth_term,
        } = self;
        if term.generation() == birth_generation {
            if term != birth_term {
                return Err(PublicationError::Mismatch);
            }
        } else {
            let row = txn
                .get(&f.renewal_operations, &term.operation().to_hex())
                .map_err(bad)?
                .cloned()
                .ok_or(PublicationError::Missing)?;
            let economy_row = txn
                .get(&f.term_economics, &term.key())
                .map_err(bad)?
                .cloned()
                .ok_or(PublicationError::Missing)?;
            let view = economy
                .renewal_audit()
                .map_err(bad)?
                .get(row.operation())
                .map_err(bad)?;
            super::super::renewal::history::verify(&row, &view)?;
            let (_, expected_term, expected_economics) = row.granted_rows()?;
            let Some(birth) = new.as_ref() else {
                return Err(PublicationError::Mismatch);
            };
            let accepted_pointer = row.binding().accepted_pointer(licence.term())?;
            if !row.binding().matches_birth(&accepted_pointer)
                || term != &expected_term
                || economy_row != expected_economics
                || row.binding().product_digest() != birth.product_digest
                || row.binding().grant() != birth.economics.grant()
                || view.offer().basis.prior_quote != birth.quote
                || view.offer().basis.prior_quote_digest != birth.quote_digest
            {
                return Err(PublicationError::Mismatch);
            }
            let prior = txn
                .get(
                    &f.paid_terms,
                    &OwnerTerm::encoded_key(licence.id(), row.binding().predecessor()),
                )
                .map_err(bad)?
                .cloned()
                .ok_or(PublicationError::Missing)?;
            let prior_economics = txn
                .get(&f.term_economics, &prior.key())
                .map_err(bad)?
                .cloned()
                .ok_or(PublicationError::Missing)?;
            row.binding()
                .check_predecessor(&accepted_pointer, &prior, &prior_economics)?;
        }
        let head = txn
            .get(&f.renewal_heads, &licence.id().key())
            .map_err(bad)?
            .cloned();
        match head {
            None if licence.term().get() == 1 => {}
            Some(head) => {
                let row = txn
                    .get(&f.renewal_operations, &head.last_operation().to_hex())
                    .map_err(bad)?
                    .cloned()
                    .ok_or(PublicationError::Missing)?;
                if head != super::super::renewal::RenewalHeadV1::for_operation(&row)?
                    || head.current() != licence.term()
                    || !row
                        .binding()
                        .matches_birth(&row.binding().accepted_pointer(licence.term())?)
                    || (row.state().pending() && !row.binding().matches_birth(licence))
                {
                    return Err(PublicationError::Mismatch);
                }
                let view = economy
                    .renewal_audit()
                    .map_err(bad)?
                    .get(row.operation())
                    .map_err(bad)?;
                super::super::renewal::history::verify(&row, &view)?;
            }
            _ => return Err(PublicationError::Mismatch),
        }
        Ok(())
    }
}

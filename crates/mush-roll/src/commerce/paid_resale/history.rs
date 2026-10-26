//! Immutable acquisition/holder proof, separate from current use eligibility.
//! Every non-birth holder is tied to the actual Economy owner's complete native
//! history. Full boot additionally proves the entire unique holder/operation chain.
use super::records::*;
use crate::commerce::{
    RevisionIdentity,
    entitlement::history::{self as acquired, Birth, optional, read},
    families::CommerceFamilies,
    paid::{PaidTermRecord, PersonalPaidLicence},
    rating::RatingFamilies,
    read::CommerceRead,
    records::{PublicationError, Record},
    roots::{RequiredRoot, RootAnchor},
};
use mush_core::{ClockRef, ObjId, World};
use mush_economy::{
    EconomyStore,
    managed_paid_resale::{PaidBirthRefV1, PaidHolderRefV1},
};

fn bad(_: impl std::fmt::Display) -> PublicationError {
    PublicationError::Mismatch
}

/// This only exists after the real birth allocation, complete purchase receipt
/// and immutable original term have been read and joined.
pub(crate) struct BirthProof {
    pub(crate) reference: PaidBirthRefV1,
    pub(crate) holder: ObjId,
    pub(crate) revision: RevisionIdentity,
    /// 17g C1a: the issuing House of a House-born licence, read from the same
    /// proved allocation row; `None` for every personal birth route. The resale
    /// fence hands it to a configured House buyer admission and nothing else.
    pub(crate) house_issuer: Option<crate::commerce::house_catalogue::HouseIssuer>,
}
pub(crate) fn birth(
    ctx: &impl CommerceRead,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
    licence: &PersonalPaidLicence,
) -> Result<BirthProof, PublicationError> {
    licence.validate()?;
    let row = acquired::resolve(ctx, f, licence.operation())?;
    let term = read(
        ctx,
        &f.paid_terms,
        format!("{}/{}", licence.id().key(), row.term().key()),
    )?;
    if row.licence() != licence.id()
        || row.charter() != licence.charter()
        || row.line() != licence.line()
    {
        return Err(PublicationError::Mismatch);
    }
    // A historical data projection is not a new licence or publication. It lets
    // the old exact birth verifier continue to compare its original bytes.
    let original = PersonalPaidLicence::original(
        row.licence(),
        row.charter(),
        row.line(),
        row.holder(),
        row.term(),
        row.operation(),
    )?;
    acquired::granted(ctx, f, ratings, &original, term)?;
    let reference = birth_reference(row)?;
    let house_issuer = match row {
        Birth::House(a) => Some(a.issuer()),
        Birth::Legacy(_) | Birth::Entitlement(_) => None,
    };
    Ok(BirthProof {
        reference,
        holder: row.holder(),
        revision: row.revision(),
        house_issuer,
    })
}
pub(crate) fn birth_reference(row: Birth<'_>) -> Result<PaidBirthRefV1, PublicationError> {
    let (route, value) = match row {
        Birth::Legacy(a) => ("legacy", serde_json::to_value(a).map_err(bad)?),
        Birth::Entitlement(a) => ("entitlement", serde_json::to_value(a).map_err(bad)?),
        Birth::House(a) => ("house", serde_json::to_value(a).map_err(bad)?),
    };
    serde_json::from_value(serde_json::json!({
        "route": route, "operation": row.operation(), "digest": digest("roll.paid_holder.birth.v1", &value)?
    })).map_err(bad)
}

/// Used by already complete birth audits to join today's holder independently
/// of their immutable original purchaser. No source/phase/current issuer gate.
pub(crate) fn current_from_birth(
    ctx: &impl CommerceRead,
    f: &CommerceFamilies,
    pointer: &PersonalPaidLicence,
    birth: Birth<'_>,
) -> Result<(), PublicationError> {
    if pointer.id() != birth.licence()
        || pointer.charter() != birth.charter()
        || pointer.line() != birth.line()
        || pointer.operation() != birth.operation()
    {
        return Err(PublicationError::Mismatch);
    }
    let Some(reference) = pointer.ownership() else {
        return if pointer.holder() == birth.holder() {
            Ok(())
        } else {
            Err(PublicationError::Mismatch)
        };
    };
    let holder = read(
        ctx,
        &f.paid_holders,
        PaidHolderRecord::key_for(pointer.id(), reference.generation),
    )?;
    let row = read(ctx, &f.paid_resales, holder.acceptance().to_hex())?;
    holder.matches_record(row)?;
    if holder.ownership()? != *reference
        || holder.holder() != pointer.holder()
        || holder.birth() != &birth_reference(birth)?
        || holder.licence() != pointer.id()
    {
        return Err(PublicationError::Mismatch);
    }
    native(ctx.world(), row)?;
    roots(ctx.world(), row, birth.revision())
}

pub(crate) fn current_in_txn(
    txn: &mut mush_core::ActionTxn<'_>,
    world: &World,
    f: &CommerceFamilies,
    pointer: &PersonalPaidLicence,
    birth: Birth<'_>,
) -> Result<(), PublicationError> {
    if pointer.id() != birth.licence()
        || pointer.charter() != birth.charter()
        || pointer.line() != birth.line()
        || pointer.operation() != birth.operation()
    {
        return Err(PublicationError::Mismatch);
    }
    let Some(reference) = pointer.ownership() else {
        return if pointer.holder() == birth.holder() {
            Ok(())
        } else {
            Err(PublicationError::Mismatch)
        };
    };
    let holder = txn
        .get(
            &f.paid_holders,
            &PaidHolderRecord::key_for(pointer.id(), reference.generation),
        )
        .map_err(bad)?
        .cloned()
        .ok_or(PublicationError::Missing)?;
    let row = txn
        .get(&f.paid_resales, &holder.acceptance().to_hex())
        .map_err(bad)?
        .ok_or(PublicationError::Missing)?;
    holder.matches_record(row)?;
    if holder.ownership()? != *reference
        || holder.holder() != pointer.holder()
        || holder.birth() != &birth_reference(birth)?
        || holder.licence() != pointer.id()
    {
        return Err(PublicationError::Mismatch);
    }
    native(world, row)?;
    roots(world, row, birth.revision())
}

fn economy(world: &World) -> Result<&EconomyStore, PublicationError> {
    if world
        .module_state_owner(std::any::TypeId::of::<EconomyStore>())
        .is_none_or(|o| o.as_str() != "economy")
    {
        return Err(PublicationError::Unavailable);
    }
    world
        .module_state::<EconomyStore>()
        .ok_or(PublicationError::Unavailable)
}

pub(crate) fn native(world: &World, row: &PaidResaleRecord) -> Result<(), PublicationError> {
    row.validate()?;
    if economy(world)?
        .paid_resale_history(row.operation())
        .map_err(bad)?
        != row.economic()
    {
        return Err(PublicationError::Mismatch);
    }
    Ok(())
}

/// Stable acceptance root: a later terminal does not rewrite the accepted
/// binding, consent or real funding/immediate-payment receipt.
pub(crate) fn acceptance_root(
    row: &PaidResaleRecord,
    revision: RevisionIdentity,
) -> Result<RequiredRoot, PublicationError> {
    row.validate()?;
    let d = digest(
        "roll.paid_resale.acceptance.v1",
        &serde_json::json!({
            "binding": row.binding(), "claims": row.claims(),
            "consent": row.economic().consent(), "acceptance": row.economic().acceptance(),
        }),
    )?;
    Ok(RequiredRoot {
        anchor: RootAnchor::PaidResaleFunding(row.operation(), d),
        identity: revision,
    })
}
pub(crate) fn terminal_root(
    row: &PaidResaleRecord,
    revision: RevisionIdentity,
) -> Result<RequiredRoot, PublicationError> {
    if row.state().terminal().is_none() {
        return Err(PublicationError::Mismatch);
    }
    Ok(RequiredRoot {
        anchor: RootAnchor::PaidResaleTerminal(row.operation(), row.digest()?),
        identity: revision,
    })
}
pub(crate) fn roots(
    world: &World,
    row: &PaidResaleRecord,
    revision: RevisionIdentity,
) -> Result<(), PublicationError> {
    let roll = world
        .module_state::<crate::RollStore>()
        .ok_or(PublicationError::Unavailable)?;
    roll.check_required_root(&acceptance_root(row, revision)?)
        .map_err(bad)?;
    if row.state().terminal().is_some() {
        roll.check_required_root(&terminal_root(row, revision)?)
            .map_err(bad)?;
    }
    Ok(())
}

/// The clock comes from the actual issued native receipt, not today's clock or
/// the holder-transfer acceptance. Old terms keep their original clock identity.
pub(crate) fn issued_clock(
    ctx: &impl CommerceRead,
    f: &CommerceFamilies,
    licence: &PersonalPaidLicence,
    term: &PaidTermRecord,
) -> Result<ClockRef, PublicationError> {
    let e = economy(ctx.world())?;
    if term.generation().get() == 1 {
        let audit = e.purchase_receipt_audit();
        audit.require_complete().map_err(bad)?;
        let stamp = audit
            .get(licence.operation())
            .map_err(bad)?
            .terminal()
            .and_then(|t| t.issued_at_ic())
            .ok_or(PublicationError::Missing)?;
        return Ok(stamp.clock.clone());
    }
    let birth = acquired::resolve(ctx, f, licence.operation())?;
    let stamp = if matches!(birth, Birth::House(_)) {
        e.house_renewal_audit()
            .map_err(bad)?
            .get(term.operation())
            .map_err(bad)?
            .history()
            .and_then(|h| h.terminal())
            .and_then(|t| t.issued_at_ic())
            .cloned()
    } else {
        e.renewal_audit()
            .map_err(bad)?
            .get(term.operation())
            .map_err(bad)?
            .history()
            .and_then(|h| h.terminal())
            .and_then(|t| t.issued_at_ic())
            .cloned()
    }
    .ok_or(PublicationError::Missing)?;
    Ok(stamp.clock)
}

/// One exact retained holder-generation proof. It does not consult the current
/// holder, current phase, sanction, issuer, parent or person lifecycle. A captured
/// IC claim can therefore retain its old dispatcher after a later transfer.
pub(crate) fn historical(
    ctx: &impl CommerceRead,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
    licence: &PersonalPaidLicence,
    reference: &PaidHolderRefV1,
) -> Result<ObjId, PublicationError> {
    reference.validate().map_err(bad)?;
    let birth = birth(ctx, f, ratings, licence)?;
    if reference.generation == 1 {
        if reference.operation != birth.reference.operation
            || reference.digest != birth.reference.digest
        {
            return Err(PublicationError::Mismatch);
        }
        return Ok(birth.holder);
    }
    let holder = read(
        ctx,
        &f.paid_holders,
        PaidHolderRecord::key_for(licence.id(), reference.generation),
    )?;
    let row = read(ctx, &f.paid_resales, holder.acceptance().to_hex())?;
    holder.matches_record(row)?;
    if &holder.ownership()? != reference
        || holder.birth() != &birth.reference
        || row.binding().birth() != &birth.reference
        || row.binding().charter() != licence.charter()
        || row.binding().line() != licence.line()
        || holder.licence() != licence.id()
    {
        return Err(PublicationError::Mismatch);
    }
    let term = read(ctx, &f.paid_terms, row.binding().term().key())?;
    acquired::granted(ctx, f, ratings, licence, term)?;
    if term != row.binding().term()
        || issued_clock(ctx, f, licence, term)? != *row.binding().term_clock()
    {
        return Err(PublicationError::Mismatch);
    }
    native(ctx.world(), row)?;
    roots(ctx.world(), row, birth.revision)?;
    Ok(holder.holder())
}

/// Current ownership is proved separately from the accepted term holder. No
/// current use/source/renewal eligibility is granted by this metadata relation.
pub(crate) fn current(
    ctx: &impl CommerceRead,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
    licence: &PersonalPaidLicence,
) -> Result<PaidHolderRefV1, PublicationError> {
    let reference = match licence.ownership() {
        Some(r) => *r,
        None => {
            let b = birth(ctx, f, ratings, licence)?;
            PaidHolderRefV1 {
                generation: 1,
                operation: b.reference.operation,
                digest: b.reference.digest,
            }
        }
    };
    if historical(ctx, f, ratings, licence, &reference)? != licence.holder() {
        return Err(PublicationError::Mismatch);
    }
    Ok(reference)
}

/// A pending resale is a live exclusion only. Terminal history remains readable
/// and later actual holder/term generations do not rewrite it.
pub(crate) fn require_not_pending(
    ctx: &impl CommerceRead,
    f: &CommerceFamilies,
    licence: &PersonalPaidLicence,
) -> Result<(), PublicationError> {
    if let Some(head) = optional(ctx, &f.paid_resale_heads, licence.id().key())? {
        let row = read(ctx, &f.paid_resales, head.current().operation.to_hex())?;
        head.matches(row)?;
        native(ctx.world(), row)?;
        if matches!(row.state(), PaidResaleState::Funded {}) {
            return Err(PublicationError::Mismatch);
        }
    }
    Ok(())
}

/// Exact settled holder acceptance for a fresh renewal observation. Birth
/// holders retain the old format and do not acquire a fabricated transfer.
pub(crate) fn renewal_basis(
    ctx: &impl CommerceRead,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
    licence: &PersonalPaidLicence,
) -> Result<Option<mush_economy::managed_paid_resale::RenewalHolderBasisV1>, PublicationError> {
    let Some(ownership) = licence.ownership() else {
        return Ok(None);
    };
    if current(ctx, f, ratings, licence)? != *ownership {
        return Err(PublicationError::Mismatch);
    }
    let holder = read(
        ctx,
        &f.paid_holders,
        PaidHolderRecord::key_for(licence.id(), ownership.generation),
    )?;
    Ok(Some(
        mush_economy::managed_paid_resale::RenewalHolderBasisV1 {
            acceptance: holder.acceptance(),
            ownership: *ownership,
        },
    ))
}

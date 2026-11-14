//! Roll/E4 half of mandatory renewal completeness. The Market paired audit is
//! an additional required owner gate before configured commerce can open.

use super::super::{
    LicenceId, RevisionIdentity, TermGeneration,
    audit::{read, scan},
    entitlement::history::{self as birth_history},
    families::CommerceFamilies,
    rating::RatingFamilies,
    records::{PublicationError, Record},
    roots::{RequiredRoot, RootAnchor},
};
use super::records::RenewalHeadV1;
use super::{Binding, Operation, Rules, TermEconomics};
use crate::RollStore;
use mush_core::ModuleCtx;
use mush_economy::EconomyStore;
use std::collections::{BTreeMap, BTreeSet};

/// Recheck a later current pointer for a historical birth allocation. This is
/// used only after the full Roll audit opened commerce; it neither substitutes
/// for that reverse inventory nor imposes current issuer eligibility.
pub(crate) fn verify_current_pointer<I: Rules>(
    ctx: &ModuleCtx<'_>,
    roll: &RollStore,
    f: &CommerceFamilies,
    licence: &super::super::paid::PersonalPaidLicence,
) -> Result<(), PublicationError> {
    if licence.term().get() <= 1 {
        return Err(PublicationError::Mismatch);
    }
    let term_key = format!("{}/{}", licence.id().key(), licence.term().key());
    let term = read(ctx, &f.paid_terms, term_key.clone())?;
    term.matches_current(licence)?;
    let economics = read(ctx, I::economics(f), term_key)?;
    let head = read(ctx, I::heads(f), licence.id().key())?;
    let last = read(ctx, I::operations(f), head.last_operation().to_hex())?;
    if head != &RenewalHeadV1::<I>::for_operation(last)?
        || head.current() != licence.term()
        || (last.state().pending() && !last.binding().matches_birth(licence))
        || last.binding().licence() != licence.id()
        || last.binding().birth_operation() != licence.operation()
    {
        return Err(PublicationError::Mismatch);
    }
    let economy = ctx
        .state::<EconomyStore>()
        .ok_or(PublicationError::Unavailable)?;
    if economy.is_frozen() {
        return Err(PublicationError::Unavailable);
    }
    let actual = I::audit(economy).map_err(|_| PublicationError::Unavailable)?;
    let last_view = actual
        .get(last.operation())
        .map_err(|_| PublicationError::Missing)?;
    super::history::verify::<I>(last, &last_view)?;
    let granted = if term.operation() == last.operation() {
        last
    } else {
        let row = read(ctx, I::operations(f), term.operation().to_hex())?;
        let view = actual
            .get(row.operation())
            .map_err(|_| PublicationError::Missing)?;
        super::history::verify::<I>(row, &view)?;
        row
    };
    let (expected_licence, expected_term, expected_economics) = granted.granted_rows()?;
    if expected_licence.id() != licence.id()
        || expected_licence.operation() != licence.operation()
        || expected_licence.term() != licence.term()
        || term != &expected_term
        || economics != &expected_economics
    {
        return Err(PublicationError::Mismatch);
    }
    for row in [last, granted] {
        for root in required::<I>(row)? {
            roll.check_required_root(&root)
                .map_err(|_| PublicationError::Mismatch)?;
        }
    }
    Ok(())
}

pub(crate) fn required<I: Rules>(
    row: &I::Operation,
) -> Result<[RequiredRoot; 2], PublicationError> {
    row.validate()?;
    Ok([
        RequiredRoot {
            anchor: I::anchor(row.operation()),
            identity: row.binding().revision(),
        },
        RequiredRoot {
            anchor: RootAnchor::Term(row.binding().licence(), row.binding().next()),
            identity: row.binding().revision(),
        },
    ])
}

/// Full bounded scans run even with zero renewal operations. Immutable E4
/// history must match both directions, including unfunded offers and closures.
pub(crate) fn audit<I: Rules>(
    ctx: &ModuleCtx<'_>,
    roll: &RollStore,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
    enabled: bool,
) -> Result<BTreeMap<RootAnchor, RevisionIdentity>, PublicationError> {
    let mut roots = BTreeMap::new();
    if !enabled {
        let operations = scan(ctx, I::operations(f), |_| Ok(()))?;
        let heads = scan(ctx, I::heads(f), |_| Ok(()))?;
        if operations != 0 || heads != 0 {
            return Err(PublicationError::Unavailable);
        }
        // An empty Roll projection is not proof that Economy has no retained
        // offers. The owner proves its actual boot inventory and exact current
        // emptiness without requiring the enabled combined-renewal latch.
        I::require_no_history(
            ctx.state::<EconomyStore>()
                .ok_or(PublicationError::Unavailable)?,
        )
        .map_err(|_| PublicationError::Unavailable)?;
        return Ok(roots);
    }
    let economy = ctx
        .state::<EconomyStore>()
        .ok_or(PublicationError::Unavailable)?;
    let actual = I::audit(economy).map_err(|_| PublicationError::Unavailable)?;
    actual
        .require_complete()
        .map_err(|_| PublicationError::Unavailable)?;
    let mut operations: BTreeMap<LicenceId, Vec<&I::Operation>> = BTreeMap::new();
    let mut seen = BTreeSet::new();
    let mut commits = BTreeSet::new();
    let mut contracts = BTreeSet::new();
    scan(ctx, I::operations(f), |row| {
        if seen.len() == super::records::MAX_OPERATIONS
            || !seen.insert(row.operation())
            || !commits.insert(row.operation())
            || !contracts.insert(row.contract())
            || row
                .state()
                .funding()
                .is_some_and(|r| !commits.insert(r.commit()))
            || row
                .state()
                .terminal_commit()
                .is_some_and(|c| !commits.insert(c))
        {
            return Err(PublicationError::Mismatch);
        }
        let view = actual
            .get(row.operation())
            .map_err(|_| PublicationError::Missing)?;
        super::history::verify::<I>(row, &view)?;
        let licence = read(ctx, &f.paid_licences, row.binding().licence().key())?;
        if let Some(basis) = row.binding().holder_basis() {
            let holder = super::super::paid_resale::history::historical(
                ctx,
                f,
                ratings,
                licence,
                &basis.ownership,
            )?;
            let captured = read(
                ctx,
                &f.paid_holders,
                super::super::paid_resale::PaidHolderRecord::key_for(
                    licence.id(),
                    basis.ownership.generation,
                ),
            )?;
            // M8c F4: by principal (the binding's acting is the renewer's).
            if !holder.same_principal(row.binding().holder())
                || captured.acceptance() != basis.acceptance
            {
                return Err(PublicationError::Mismatch);
            }
        } else if !birth_history::resolve(ctx, f, licence.operation())?
            .holder()
            .same_principal(row.binding().holder())
        {
            return Err(PublicationError::Mismatch);
        }

        let predecessor = read(
            ctx,
            &f.paid_terms,
            format!(
                "{}/{}",
                row.binding().licence().key(),
                row.binding().predecessor().key()
            ),
        )?;
        let prior = birth_history::granted(ctx, f, ratings, licence, predecessor)?;
        I::check_birth(&prior.birth(), row.binding(), view.offer())?;
        for required in required::<I>(row)? {
            roll.check_required_root(&required)
                .map_err(|_| PublicationError::Mismatch)?;
            // Exact retries share the same already reserved next-generation
            // term root. A different identity at that key always refuses.
            if roots
                .insert(required.anchor, required.identity)
                .is_some_and(|old| old != required.identity)
            {
                return Err(PublicationError::Mismatch);
            }
        }
        operations
            .entry(row.binding().licence())
            .or_default()
            .push(read(ctx, I::operations(f), row.key())?);
        Ok(())
    })?;
    if seen.len() != actual.len() {
        return Err(PublicationError::Mismatch);
    }
    for view in actual.offers() {
        let view = view.map_err(|_| PublicationError::Unavailable)?;
        let row = read(ctx, I::operations(f), view.operation().to_hex())?;
        super::history::verify::<I>(row, &view)?;
    }
    let mut heads = BTreeMap::<LicenceId, &RenewalHeadV1<I>>::new();
    scan(ctx, I::heads(f), |row| {
        if heads
            .insert(row.licence(), read(ctx, I::heads(f), row.key())?)
            .is_some()
            || !operations.contains_key(&row.licence())
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    })?;
    if heads.len() != operations.len() {
        return Err(PublicationError::Mismatch);
    }
    let mut terms = BTreeMap::new();
    scan(ctx, &f.paid_terms, |row| {
        if operations.contains_key(&row.licence()) {
            let set = terms.entry(row.licence()).or_insert_with(BTreeMap::new);
            if set
                .insert(row.generation(), read(ctx, &f.paid_terms, row.key())?)
                .is_some()
            {
                return Err(PublicationError::Mismatch);
            }
        } else if row.generation().get() != 1 {
            let licence = read(ctx, &f.paid_licences, row.licence().key())?;
            // The other issuer's disjoint audit proves every actual generation of its own.
            if !I::other_issuers_birth(&birth_history::resolve(ctx, f, licence.operation())?) {
                return Err(PublicationError::Mismatch);
            }
        }
        Ok(())
    })?;
    let mut economics = BTreeMap::new();
    scan(ctx, I::economics(f), |row| {
        if operations.contains_key(&row.licence()) {
            let set = economics.entry(row.licence()).or_insert_with(BTreeMap::new);
            if set
                .insert(row.generation(), read(ctx, I::economics(f), row.key())?)
                .is_some()
            {
                return Err(PublicationError::Mismatch);
            }
        } else if row.generation().get() != 1 {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    })?;
    for (id, rows) in operations {
        let licence = read(ctx, &f.paid_licences, id.key())?;
        let line = I::line(ctx, f, licence)?;
        super::chain::verify::<I>(
            licence,
            line,
            heads.get(&id).copied(),
            &rows,
            terms.get(&id).ok_or(PublicationError::Missing)?,
            economics.get(&id).ok_or(PublicationError::Missing)?,
        )?;
        let one = TermGeneration::new(1).map_err(|_| PublicationError::Invalid)?;
        let birth = read(ctx, &f.paid_terms, format!("{}/{}", id.key(), one.key()))?;
        birth_history::granted(ctx, f, ratings, licence, birth)?;
    }
    Ok(roots)
}

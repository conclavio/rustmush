//! Roll/E4 half of mandatory renewal completeness. The Market paired audit is
//! an additional required owner gate before configured commerce can open.

use super::super::{
    LicenceId, RevisionIdentity, TermGeneration,
    audit::{read, scan},
    entitlement::history::{self as birth_history, Birth},
    families::CommerceFamilies,
    rating::RatingFamilies,
    records::{PublicationError, Record},
    roots::{RequiredRoot, RootAnchor},
};
use super::{RenewalHeadV1, RenewalOperationV1};
use crate::RollStore;
use mush_core::ModuleCtx;
use mush_economy::EconomyStore;
use std::collections::{BTreeMap, BTreeSet};

/// Recheck a later current pointer for a historical birth allocation. This is
/// used only after the full Roll audit opened commerce; it neither substitutes
/// for that reverse inventory nor imposes current issuer eligibility.
pub(crate) fn verify_current_pointer(
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
    let economics = read(ctx, &f.term_economics, term_key)?;
    let head = read(ctx, &f.renewal_heads, licence.id().key())?;
    let last = read(ctx, &f.renewal_operations, head.last_operation().to_hex())?;
    if head != &RenewalHeadV1::for_operation(last)?
        || head.current() != licence.term()
        || !last.binding().matches_birth(licence)
    {
        return Err(PublicationError::Mismatch);
    }
    let economy = ctx
        .state::<EconomyStore>()
        .ok_or(PublicationError::Unavailable)?;
    if economy.is_frozen() {
        return Err(PublicationError::Unavailable);
    }
    let actual = economy
        .renewal_audit()
        .map_err(|_| PublicationError::Unavailable)?;
    let last_view = actual
        .get(last.operation())
        .map_err(|_| PublicationError::Missing)?;
    last.verify_history(&last_view)?;
    let granted = if term.operation() == last.operation() {
        last
    } else {
        let row = read(ctx, &f.renewal_operations, term.operation().to_hex())?;
        let view = actual
            .get(row.operation())
            .map_err(|_| PublicationError::Missing)?;
        row.verify_history(&view)?;
        row
    };
    let (expected_licence, expected_term, expected_economics) = granted.granted_rows()?;
    if licence != &expected_licence || term != &expected_term || economics != &expected_economics {
        return Err(PublicationError::Mismatch);
    }
    for row in [last, granted] {
        for root in required(row)? {
            roll.check_required_root(&root)
                .map_err(|_| PublicationError::Mismatch)?;
        }
    }
    Ok(())
}

pub(crate) fn required(row: &RenewalOperationV1) -> Result<[RequiredRoot; 2], PublicationError> {
    row.validate()?;
    Ok([
        RequiredRoot {
            anchor: RootAnchor::RenewalOperation(row.operation()),
            identity: row.binding.revision,
        },
        RequiredRoot {
            anchor: RootAnchor::Term(row.binding.licence, row.binding.next),
            identity: row.binding.revision,
        },
    ])
}

/// Full bounded scans run even with zero renewal operations. Immutable E4
/// history must match both directions, including unfunded offers and closures.
pub(crate) fn audit(
    ctx: &ModuleCtx<'_>,
    roll: &RollStore,
    f: &CommerceFamilies,
    ratings: &RatingFamilies,
    enabled: bool,
) -> Result<BTreeMap<RootAnchor, RevisionIdentity>, PublicationError> {
    let mut roots = BTreeMap::new();
    if !enabled {
        let operations = scan(ctx, &f.renewal_operations, |_| Ok(()))?;
        let heads = scan(ctx, &f.renewal_heads, |_| Ok(()))?;
        if operations != 0 || heads != 0 {
            return Err(PublicationError::Unavailable);
        }
        // An empty Roll projection is not proof that Economy has no retained
        // offers. The owner proves its actual boot inventory and exact current
        // emptiness without requiring the enabled combined-renewal latch.
        ctx.state::<EconomyStore>()
            .ok_or(PublicationError::Unavailable)?
            .require_no_renewal_history()
            .map_err(|_| PublicationError::Unavailable)?;
        return Ok(roots);
    }
    let economy = ctx
        .state::<EconomyStore>()
        .ok_or(PublicationError::Unavailable)?;
    let actual = economy
        .renewal_audit()
        .map_err(|_| PublicationError::Unavailable)?;
    actual
        .require_complete()
        .map_err(|_| PublicationError::Unavailable)?;
    let mut operations: BTreeMap<LicenceId, Vec<&RenewalOperationV1>> = BTreeMap::new();
    let mut seen = BTreeSet::new();
    let mut commits = BTreeSet::new();
    let mut contracts = BTreeSet::new();
    scan(ctx, &f.renewal_operations, |row| {
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
        super::history::verify(row, &view)?;
        let licence = read(ctx, &f.paid_licences, row.binding.licence.key())?;
        let predecessor = read(
            ctx,
            &f.paid_terms,
            format!(
                "{}/{}",
                row.binding.licence.key(),
                row.binding.predecessor.key()
            ),
        )?;
        let prior = birth_history::granted(ctx, f, ratings, licence, predecessor)?;
        let Birth::Entitlement(birth) = prior.birth() else {
            return Err(PublicationError::Mismatch);
        };
        if row.binding.product_digest != birth.product_digest
            || row.binding.grant != *birth.economics.grant()
            || view.offer().basis.prior_quote != birth.quote
            || view.offer().basis.prior_quote_digest != birth.quote_digest
        {
            return Err(PublicationError::Mismatch);
        }
        for required in required(row)? {
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
            .entry(row.binding.licence)
            .or_default()
            .push(read(ctx, &f.renewal_operations, row.key())?);
        Ok(())
    })?;
    if seen.len() != actual.len() {
        return Err(PublicationError::Mismatch);
    }
    for view in actual.offers() {
        let view = view.map_err(|_| PublicationError::Unavailable)?;
        let row = read(ctx, &f.renewal_operations, view.operation().to_hex())?;
        super::history::verify(row, &view)?;
    }
    let mut heads = BTreeMap::<LicenceId, &RenewalHeadV1>::new();
    scan(ctx, &f.renewal_heads, |row| {
        if heads
            .insert(row.licence(), read(ctx, &f.renewal_heads, row.key())?)
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
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    })?;
    let mut economics = BTreeMap::new();
    scan(ctx, &f.term_economics, |row| {
        if operations.contains_key(&row.licence()) {
            let set = economics.entry(row.licence()).or_insert_with(BTreeMap::new);
            if set
                .insert(row.generation(), read(ctx, &f.term_economics, row.key())?)
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
        super::chain::verify(
            licence,
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
